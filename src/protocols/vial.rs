use super::{
    kle_parser, KeyboardDefinition, KeyboardProtocol, RawHidSubscription, SubscriptionSender,
};
use crate::layout_key::LayoutKey;
use crate::qmk_keycode_labels::get_layout_key;
use hidapi::{HidApi, HidDevice};
use std::error::Error;
use std::ffi::CString;

const VIAL_PREFIX: u8 = 0xFE;
const VIA_USAGE_PAGE: u16 = 0xff60;
const VIA_PROTOCOL_ALPHA: u16 = 7;
const VIA_PROTOCOL_BETA: u16 = 8;
const RAW_EPSIZE: usize = 32;
const VIA_DATA_BUFFER_SIZE: usize = 28;

#[repr(u8)]
enum VialCommand {
    KeyboardId = 0x00,
    Size = 0x01,
    Def = 0x02,
}

pub struct VialProtocol {
    api: VialApi,
    definition: KeyboardDefinition,
    keyboard_uid: [u8; 8],
    hid_path: CString,
}

struct VialApi {
    device: HidDevice,
    protocol_version: u16,
}

impl VialApi {
    fn new(device: HidDevice) -> Result<Self, Box<dyn Error>> {
        let mut api = Self {
            device,
            protocol_version: 0,
        };
        let response = api.command(0x01, &[])?;
        api.protocol_version = u16::from_be_bytes([response[1], response[2]]);
        Ok(api)
    }

    fn send(&self, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
        if bytes.len() > RAW_EPSIZE {
            return Err("VIA message exceeds the raw HID report size".into());
        }
        let mut report = vec![0; RAW_EPSIZE + 1];
        report[1..=bytes.len()].copy_from_slice(bytes);
        let written = self.device.write(&report)?;
        if written != report.len() {
            return Err(format!("VIA wrote {written} of {} bytes", report.len()).into());
        }
        Ok(())
    }

    fn read(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut response = vec![0; RAW_EPSIZE];
        self.device.read(&mut response)?;
        Ok(response)
    }

    fn read_event(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut response = vec![0; RAW_EPSIZE];
        self.device.read_timeout(&mut response, 200)?;
        Ok(response)
    }

    fn command(&self, command: u8, data: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut request = Vec::with_capacity(data.len() + 1);
        request.push(command);
        request.extend_from_slice(data);
        self.send(&request)?;
        let response = self.read()?;
        if !response.starts_with(&request) {
            return Err(format!("Unexpected VIA response for command 0x{command:02x}").into());
        }
        Ok(response)
    }

    fn layer_count(&self) -> Result<usize, Box<dyn Error>> {
        if self.protocol_version >= VIA_PROTOCOL_BETA {
            Ok(self.command(0x11, &[])?[1] as usize)
        } else {
            Ok(4)
        }
    }

    fn read_raw_matrix(
        &self,
        rows: usize,
        cols: usize,
        layer: u8,
    ) -> Result<Vec<u16>, Box<dyn Error>> {
        let length = rows * cols;
        if self.protocol_version == VIA_PROTOCOL_ALPHA {
            let mut result = Vec::with_capacity(length);
            for index in 0..length {
                let row = (index / cols) as u8;
                let col = (index % cols) as u8;
                let response = self.command(0x04, &[layer, row, col])?;
                result.push(u16::from_be_bytes([response[4], response[5]]));
            }
            return Ok(result);
        }
        if self.protocol_version < VIA_PROTOCOL_BETA {
            return Err(format!("Unsupported VIA protocol {}", self.protocol_version).into());
        }

        let mut result = Vec::with_capacity(length);
        for key_offset in (0..length).step_by(VIA_DATA_BUFFER_SIZE / 2) {
            let key_count = (length - key_offset).min(VIA_DATA_BUFFER_SIZE / 2);
            let byte_offset = layer as usize * length * 2 + key_offset * 2;
            let [offset_hi, offset_lo] = (byte_offset as u16).to_be_bytes();
            let response = self.command(0x12, &[offset_hi, offset_lo, (key_count * 2) as u8])?;
            result.extend(
                response[4..4 + key_count * 2]
                    .chunks_exact(2)
                    .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]])),
            );
        }
        Ok(result)
    }
}

impl VialProtocol {
    pub fn connect_expected(
        vid: u16,
        pid: u16,
        expected_uid: Option<[u8; 8]>,
        selected_hid_path: Option<&[u8]>,
    ) -> Result<Self, Box<dyn Error>> {
        let hid = HidApi::new()?;
        let matching_paths: Vec<_> = hid
            .device_list()
            .filter(|device| {
                device.vendor_id() == vid
                    && device.product_id() == pid
                    && device.usage_page() == VIA_USAGE_PAGE
            })
            .map(|device| device.path().to_owned())
            .filter(|path| selected_hid_path.is_none_or(|selected| path.to_bytes() == selected))
            .collect();
        if matching_paths.is_empty() {
            return Err(format!("Could not find Vial keyboard {vid:04x}:{pid:04x}").into());
        }
        if expected_uid.is_none() && matching_paths.len() != 1 {
            return Err(format!(
                "Cannot safely select Vial keyboard: found {} devices with {vid:04x}:{pid:04x}",
                matching_paths.len()
            )
            .into());
        }

        for path in matching_paths {
            let candidate = (|| -> Result<_, Box<dyn Error>> {
                let device = hid.open_path(&path)?;
                let api = VialApi::new(device)?;
                let (protocol_version, keyboard_uid) = Self::get_keyboard_id(&api)?;
                Ok((api, protocol_version, keyboard_uid))
            })();
            let (api, protocol_version, keyboard_uid) = match candidate {
                Ok(candidate) => candidate,
                Err(error) if expected_uid.is_some() => {
                    eprintln!(
                        "Skipping Vial candidate {}: {error}",
                        path.to_string_lossy()
                    );
                    continue;
                }
                Err(error) => return Err(error),
            };
            if protocol_version == 0 {
                if expected_uid.is_none() {
                    return Err("Device does not support VIAL protocol".into());
                }
                continue;
            }
            if expected_uid.is_none_or(|expected| expected == keyboard_uid) {
                return Self::init_from_api(api, path, vid, pid, keyboard_uid);
            }
        }
        Err("Could not find the saved Vial keyboard UID among connected devices".into())
    }

    fn init_from_api(
        api: VialApi,
        hid_path: CString,
        vid: u16,
        pid: u16,
        keyboard_uid: [u8; 8],
    ) -> Result<Self, Box<dyn Error>> {
        let definition = Self::fetch_definition(&api, vid, pid)?;

        Ok(Self {
            api,
            definition,
            keyboard_uid,
            hid_path,
        })
    }

    pub fn keyboard_uid(&self) -> [u8; 8] {
        self.keyboard_uid
    }

    fn vial_command(
        api: &VialApi,
        cmd: VialCommand,
        data: &[u8],
    ) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut msg = vec![0u8; 32];
        msg[0] = VIAL_PREFIX;
        msg[1] = cmd as u8;

        let copy_len = data.len().min(30);
        msg[2..2 + copy_len].copy_from_slice(&data[..copy_len]);

        api.send(&msg)
            .map_err(|e| format!("VIAL write error: {e}"))?;

        api.read()
            .map_err(|e| format!("VIAL read error: {e}").into())
    }

    fn vial_get_def_block(api: &VialApi, block: u32) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut msg = vec![0u8; 32];
        msg[0] = VIAL_PREFIX;
        msg[1] = VialCommand::Def as u8;
        msg[2..6].copy_from_slice(&block.to_le_bytes());

        api.send(&msg)
            .map_err(|e| format!("VIAL write error: {e}"))?;

        api.read()
            .map_err(|e| format!("VIAL read error: {e}").into())
    }

    fn get_keyboard_id(api: &VialApi) -> Result<(u32, [u8; 8]), Box<dyn Error>> {
        let response = Self::vial_command(api, VialCommand::KeyboardId, &[])?;

        let protocol_version =
            u32::from_le_bytes([response[0], response[1], response[2], response[3]]);

        let mut uid = [0u8; 8];
        uid.copy_from_slice(&response[4..12]);

        Ok((protocol_version, uid))
    }

    fn get_definition_size(api: &VialApi) -> Result<u32, Box<dyn Error>> {
        let response = Self::vial_command(api, VialCommand::Size, &[])?;
        let size = u32::from_le_bytes([response[0], response[1], response[2], response[3]]);
        Ok(size)
    }

    fn fetch_definition(
        api: &VialApi,
        vid: u16,
        pid: u16,
    ) -> Result<KeyboardDefinition, Box<dyn Error>> {
        let size = Self::get_definition_size(api)? as usize;

        if size == 0 {
            return Err("VIAL definition size is 0".into());
        }

        // Fetch compressed definition in chunks
        let mut compressed = Vec::with_capacity(size);
        let mut block: u32 = 0;

        while compressed.len() < size {
            let response = Self::vial_get_def_block(api, block)?;

            let remaining = size - compressed.len();
            let chunk_size = remaining.min(32);
            compressed.extend_from_slice(&response[..chunk_size]);

            block += 1;
        }

        let mut decompressed = Vec::new();
        {
            let mut cursor = std::io::Cursor::new(&compressed);
            lzma_rs::xz_decompress(&mut cursor, &mut decompressed)
                .map_err(|e| format!("Failed to decompress VIAL definition: {e}"))?;
        }

        let json_str = String::from_utf8(decompressed)
            .map_err(|e| format!("VIAL definition is not valid UTF-8: {e}"))?;

        let json: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| format!("Failed to parse VIAL definition JSON: {e}"))?;

        kle_parser::parse_vial_definition(&json, vid, pid)
    }
}

impl KeyboardProtocol for VialProtocol {
    fn get_layout_definition(&self) -> &KeyboardDefinition {
        &self.definition
    }

    fn get_layer_count(&self) -> Result<usize, Box<dyn Error>> {
        self.api
            .layer_count()
            .map_err(|e| format!("Failed to get layer count: {e}").into())
    }

    fn read_all_keys(
        &self,
        layers: usize,
        rows: usize,
        cols: usize,
    ) -> Vec<Vec<Vec<Option<LayoutKey>>>> {
        let mut keys = vec![vec![vec![None; cols]; rows]; layers];
        for (layer, layer_keys) in keys.iter_mut().enumerate().take(layers) {
            if let Ok(raw_matrix) = self.api.read_raw_matrix(rows, cols, layer as u8) {
                for (i, &keycode) in raw_matrix.iter().enumerate() {
                    let row = i / cols;
                    let col = i % cols;
                    layer_keys[row][col] = get_layout_key(keycode);
                }
            }
        }

        keys
    }

    fn hid_read(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        self.api
            .read_event()
            .map_err(|e| format!("HID read error: {e}").into())
    }

    fn subscription_sender(&self) -> Option<Box<dyn SubscriptionSender>> {
        RawHidSubscription::open_path(&self.hid_path)
    }
}
