use super::layout_geometry::flattened_top_left_after_center_rotation;
use super::zmk_rpc::{self, ZmkData, ZmkTransport};
use super::{Key, KeyboardDefinition, KeyboardLayout, KeyboardProtocol, Reopener};
use crate::layout_key::LayoutKey;
use hidapi::{HidApi, HidDevice};
use std::error::Error;
use std::sync::Arc;
use std::time::{Duration, Instant};

type LayerKeys3d = Vec<Vec<Vec<Option<LayoutKey>>>>;
const ZMK_USAGE_PAGE: u16 = 0xff60;

struct ZmkLayout {
    definition: KeyboardDefinition,
    layout_keys: LayerKeys3d,
    layer_count: usize,
    hid_serial_number: Option<String>,
}

pub struct ZmkProtocol {
    hid_device: HidDevice,
    layout: Arc<ZmkLayout>,
}

struct ZmkReopener {
    layout: Arc<ZmkLayout>,
}

impl Reopener for ZmkReopener {
    fn reopen(&self) -> Result<Box<dyn KeyboardProtocol>, Box<dyn Error>> {
        Ok(Box::new(ZmkProtocol::open_hid(Arc::clone(&self.layout))?))
    }
}

impl ZmkProtocol {
    pub fn connect_live(
        vid: u16,
        pid: u16,
        transport: &ZmkTransport,
        hid_serial_number: Option<String>,
    ) -> Result<Self, Box<dyn Error>> {
        let zmk_data = zmk_rpc::fetch_zmk_data(transport)?;
        let (definition, layout_keys, layer_count) = build_from_zmk_data(vid, pid, zmk_data)?;
        Self::open_hid(Arc::new(ZmkLayout {
            definition,
            layout_keys,
            layer_count,
            hid_serial_number,
        }))
    }

    fn open_hid(layout: Arc<ZmkLayout>) -> Result<Self, Box<dyn Error>> {
        let (vid, pid) = (layout.definition.vid, layout.definition.pid);
        wait_for_hid_reappearance(
            vid,
            pid,
            ZMK_USAGE_PAGE,
            layout.hid_serial_number.as_deref(),
            Duration::from_secs(8),
        )
        .map_err(std::io::Error::other)?;
        let hid_device =
            open_zmk_hid(vid, pid, layout.hid_serial_number.as_deref()).map_err(|e| {
                std::io::Error::other(format!(
                    "Failed to connect HID ({vid:04x}:{pid:04x}) after reappearance: {e}"
                ))
            })?;

        Ok(Self { hid_device, layout })
    }
}

fn open_zmk_hid(
    vid: u16,
    pid: u16,
    expected_serial_number: Option<&str>,
) -> Result<HidDevice, String> {
    let api = HidApi::new().map_err(|e| format!("hidapi init failed: {e}"))?;
    let mut matches = api.device_list().filter(|device| {
        device.vendor_id() == vid
            && device.product_id() == pid
            && device.usage_page() == ZMK_USAGE_PAGE
    });
    let path = if let Some(expected_serial_number) = expected_serial_number {
        matches
            .find(|device| {
                serial_number_matches(Some(expected_serial_number), device.serial_number())
            })
            .map(|device| device.path().to_owned())
            .ok_or_else(|| {
                format!(
                    "could not find HID interface for saved serial number {expected_serial_number}"
                )
            })?
    } else {
        let first = matches.next().ok_or_else(|| {
            format!(
                "could not find HID interface for {:04x}:{:04x} usage 0x{:04x}",
                vid, pid, ZMK_USAGE_PAGE
            )
        })?;
        let path = first.path().to_owned();
        if matches.next().is_some() {
            return Err(format!(
                "cannot safely select ZMK HID interface: multiple devices use {vid:04x}:{pid:04x}"
            ));
        }
        path
    };

    api.open_path(&path).map_err(|e| e.to_string())
}

fn wait_for_hid_reappearance(
    vid: u16,
    pid: u16,
    usage_page: u16,
    expected_serial_number: Option<&str>,
    timeout: Duration,
) -> Result<(), String> {
    // On Linux BLE, the HID node can temporarily disappear while HoG/GATT activity settles; wait
    // for the matching HID interface to reappear before reconnecting via hidapi.
    let deadline = Instant::now() + timeout;
    let mut device_present_without_usage = false;
    while Instant::now() < deadline {
        let api = HidApi::new().map_err(|e| format!("hidapi init failed: {e}"))?;
        let mut matched = false;
        for d in api.device_list() {
            if d.vendor_id() == vid && d.product_id() == pid {
                if d.usage_page() == usage_page
                    && serial_number_matches(expected_serial_number, d.serial_number())
                {
                    matched = true;
                    break;
                }
                device_present_without_usage = true;
            }
        }
        if matched {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(150));
    }

    if device_present_without_usage {
        return Err("Please re-pair the keyboard to refresh the HID descriptor.".to_string());
    }

    Err(format!(
        "HID interface did not reappear in {} ms for {:04x}:{:04x} usage 0x{:04x}",
        timeout.as_millis(),
        vid,
        pid,
        usage_page
    ))
}

fn serial_number_matches(expected: Option<&str>, observed: Option<&str>) -> bool {
    expected.is_none_or(|expected| observed == Some(expected))
}

impl KeyboardProtocol for ZmkProtocol {
    fn get_layout_definition(&self) -> &KeyboardDefinition {
        &self.layout.definition
    }

    fn get_layer_count(&self) -> Result<usize, Box<dyn Error>> {
        Ok(self.layout.layer_count)
    }

    fn read_all_keys(&self, _layers: usize, _rows: usize, _cols: usize) -> LayerKeys3d {
        self.layout.layout_keys.clone()
    }

    fn hid_read(&self) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut buffer = vec![0; 32];
        self.hid_device
            .read_timeout(&mut buffer, 200)
            .map_err(|e| format!("HID read error: {e}").into())
            .map(|_| buffer)
    }

    fn reopener(&self) -> Option<Arc<dyn Reopener>> {
        Some(Arc::new(ZmkReopener {
            layout: Arc::clone(&self.layout),
        }))
    }
}

fn build_from_zmk_data(
    vid: u16,
    pid: u16,
    data: ZmkData,
) -> Result<(KeyboardDefinition, LayerKeys3d, usize), Box<dyn Error>> {
    const ACTIVE_LAYOUT_NAME: &str = "active physical layout";

    let active_idx = data.physical_layouts.active_layout_index as usize;
    let proto_layouts = &data.physical_layouts.layouts;

    if proto_layouts.is_empty() {
        return Err("Device has no physical layouts".into());
    }

    let active_layout = proto_layouts
        .get(active_idx)
        .ok_or_else(|| format!("Invalid active layout index: {active_idx}"))?;
    let active_keys: Vec<Key> = active_layout
        .keys
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let w = k.width as f32 / 100.0;
            let h = k.height as f32 / 100.0;

            let x = k.x as f32 / 100.0;
            let y = k.y as f32 / 100.0;

            // Position is where the key's center lands after rotating around the pivot;
            // the rotation itself is applied at render time via `r`.
            let angle_deg = k.r as f32 / 100.0;
            let pivot_x = if k.rx == 0 { k.x } else { k.rx } as f32 / 100.0;
            let pivot_y = if k.ry == 0 { k.y } else { k.ry } as f32 / 100.0;
            let (x, y) =
                flattened_top_left_after_center_rotation(x, y, w, h, angle_deg, pivot_x, pivot_y);

            Key {
                row: 0,
                col: i,
                x,
                y,
                w,
                h,
                r: angle_deg,
            }
        })
        .collect();
    let num_keys = active_keys.len();

    let definition = KeyboardDefinition {
        vid,
        pid,
        rows: 1,
        cols: num_keys,
        layouts: vec![KeyboardLayout {
            name: ACTIVE_LAYOUT_NAME.to_string(),
            keys: active_keys,
        }],
    };

    let layer_count = data.layer_count;
    let active_key_count = num_keys;
    let mut layout_keys_3d = Vec::with_capacity(layer_count);

    for layer_keys in &data.layout_keys {
        let mut row = vec![None; num_keys];

        for (pos, key) in layer_keys.iter().enumerate() {
            if pos >= active_key_count {
                break;
            }
            if pos < num_keys {
                row[pos] = key.clone();
            }
        }

        layout_keys_3d.push(vec![row]);
    }

    Ok((definition, layout_keys_3d, layer_count))
}

#[cfg(test)]
mod tests {
    use super::serial_number_matches;

    #[test]
    fn exact_serial_wait_ignores_other_identical_keyboards() {
        assert!(serial_number_matches(None, Some("office")));
        assert!(serial_number_matches(Some("home"), Some("home")));
        assert!(!serial_number_matches(Some("home"), Some("office")));
        assert!(!serial_number_matches(Some("home"), None));
    }
}
