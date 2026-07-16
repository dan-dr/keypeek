use crate::ui_wake::UiWake;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const RESUME_DEBOUNCE: Duration = Duration::from_secs(2);

#[derive(Clone)]
pub struct ResumeSignal {
    inner: Arc<ResumeSignalInner>,
}

struct ResumeSignalInner {
    requested: AtomicBool,
    last_request: Mutex<Option<Instant>>,
    ui_wake: Mutex<Option<UiWake>>,
}

impl ResumeSignal {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ResumeSignalInner {
                requested: AtomicBool::new(false),
                last_request: Mutex::new(None),
                ui_wake: Mutex::new(None),
            }),
        }
    }

    fn attach_ui_wake(&self, ui_wake: UiWake) {
        *self.inner.ui_wake.lock().unwrap() = Some(ui_wake);
    }

    fn request(&self) {
        let now = Instant::now();
        let mut last_request = self.inner.last_request.lock().unwrap();
        if last_request
            .as_ref()
            .is_some_and(|last| now.duration_since(*last) < RESUME_DEBOUNCE)
        {
            return;
        }
        *last_request = Some(now);
        drop(last_request);

        self.inner.requested.store(true, Ordering::Release);
        let ui_wake = self.inner.ui_wake.lock().unwrap().clone();
        if let Some(ui_wake) = ui_wake {
            ui_wake.request_repaint();
        }
    }

    fn take(&self) -> bool {
        self.inner.requested.swap(false, Ordering::AcqRel)
    }

    fn is_requested(&self) -> bool {
        self.inner.requested.load(Ordering::Acquire)
    }

    fn clear(&self) {
        self.inner.requested.store(false, Ordering::Release);
    }
}

pub struct ResumeMonitor {
    signal: ResumeSignal,
    #[cfg(target_os = "macos")]
    _registration: macos::Registration,
    #[cfg(target_os = "windows")]
    _registration: Option<windows::Registration>,
}

impl ResumeMonitor {
    pub fn install_eframe(
        cc: &eframe::CreationContext<'_>,
        signal: ResumeSignal,
        ui_wake: UiWake,
    ) -> Self {
        signal.attach_ui_wake(ui_wake);

        #[cfg(target_os = "macos")]
        let registration = macos::Registration::new(signal.clone());
        #[cfg(target_os = "windows")]
        let registration = windows::Registration::new(cc);
        #[cfg(not(target_os = "windows"))]
        let _ = cc;
        #[cfg(target_os = "linux")]
        linux::install(signal.clone());

        Self {
            signal,
            #[cfg(target_os = "macos")]
            _registration: registration,
            #[cfg(target_os = "windows")]
            _registration: registration,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn install_headless(signal: ResumeSignal, ui_wake: UiWake) -> Self {
        signal.attach_ui_wake(ui_wake);
        linux::install(signal.clone());
        Self { signal }
    }

    pub fn take_requested(&self) -> bool {
        self.signal.take()
    }

    pub fn is_requested(&self) -> bool {
        self.signal.is_requested()
    }

    pub fn clear_requested(&self) {
        self.signal.clear();
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::ResumeSignal;
    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceScreensDidWakeNotification,
        NSWorkspaceSessionDidBecomeActiveNotification,
    };
    use objc2_foundation::{
        NSNotification, NSNotificationCenter, NSObjectProtocol, NSOperationQueue,
    };
    use std::ptr::NonNull;

    pub(super) struct Registration {
        center: Retained<NSNotificationCenter>,
        observers: Vec<Retained<ProtocolObject<dyn NSObjectProtocol>>>,
    }

    impl Registration {
        pub(super) fn new(signal: ResumeSignal) -> Self {
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            let observers = [
                unsafe { NSWorkspaceDidWakeNotification },
                unsafe { NSWorkspaceScreensDidWakeNotification },
                unsafe { NSWorkspaceSessionDidBecomeActiveNotification },
            ]
            .into_iter()
            .map(|name| {
                let signal = signal.clone();
                let block = RcBlock::new(move |_notification: NonNull<NSNotification>| {
                    signal.request();
                });
                unsafe {
                    center.addObserverForName_object_queue_usingBlock(
                        Some(name),
                        None,
                        Some(&NSOperationQueue::mainQueue()),
                        &block,
                    )
                }
            })
            .collect();

            Self { center, observers }
        }
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            for observer in &self.observers {
                let observer: &ProtocolObject<dyn NSObjectProtocol> = observer;
                let observer: &AnyObject = observer.as_ref();
                unsafe { self.center.removeObserver(observer) };
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub(super) mod windows {
    use super::ResumeSignal;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::ffi::c_void;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::RemoteDesktop::{
        WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE,
        WTS_SESSION_UNLOCK,
    };
    use winit::platform::windows::EventLoopBuilderExtWindows;

    pub fn configure_event_loop(
        builder: &mut eframe::EventLoopBuilder<eframe::UserEvent>,
        signal: ResumeSignal,
    ) {
        builder.with_msg_hook(move |raw_message| {
            let message = unsafe { &*raw_message.cast::<MSG>() };
            let resumed = message.message == WM_POWERBROADCAST
                && matches!(
                    message.wParam.0 as u32,
                    PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND
                );
            let unlocked = message.message == WM_WTSSESSION_CHANGE
                && message.wParam.0 as u32 == WTS_SESSION_UNLOCK;
            if resumed || unlocked {
                signal.request();
            }
            false
        });
    }

    pub(super) struct Registration(HWND);

    impl Registration {
        pub(super) fn new(cc: &eframe::CreationContext<'_>) -> Option<Self> {
            let handle = cc.window_handle().ok()?;
            let RawWindowHandle::Win32(handle) = handle.as_raw() else {
                return None;
            };
            let hwnd = HWND(handle.hwnd.get() as *mut c_void);
            match unsafe { WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) } {
                Ok(()) => Some(Self(hwnd)),
                Err(error) => {
                    eprintln!("KeyPeek: could not monitor Windows session unlock ({error})");
                    None
                }
            }
        }
    }

    impl Drop for Registration {
        fn drop(&mut self) {
            let _ = unsafe { WTSUnRegisterSessionNotification(self.0) };
        }
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::ResumeSignal;
    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::OwnedObjectPath;

    const LOGIN_DESTINATION: &str = "org.freedesktop.login1";
    const LOGIN_MANAGER_PATH: &str = "/org/freedesktop/login1";
    const LOGIN_MANAGER_INTERFACE: &str = "org.freedesktop.login1.Manager";
    const LOGIN_SESSION_INTERFACE: &str = "org.freedesktop.login1.Session";

    pub(super) fn install(signal: ResumeSignal) {
        let wake_signal = signal.clone();
        std::thread::spawn(move || {
            if let Err(error) = listen_for_wake(wake_signal) {
                eprintln!("KeyPeek: Linux wake monitoring unavailable ({error})");
            }
        });
        std::thread::spawn(move || {
            if let Err(error) = listen_for_unlock(signal) {
                eprintln!("KeyPeek: Linux unlock monitoring unavailable ({error})");
            }
        });
    }

    fn listen_for_wake(signal: ResumeSignal) -> zbus::Result<()> {
        let connection = Connection::system()?;
        let manager = Proxy::new(
            &connection,
            LOGIN_DESTINATION,
            LOGIN_MANAGER_PATH,
            LOGIN_MANAGER_INTERFACE,
        )?;
        for message in manager.receive_signal("PrepareForSleep")? {
            let preparing_for_sleep: bool = message.body().deserialize()?;
            if !preparing_for_sleep {
                signal.request();
            }
        }
        Ok(())
    }

    fn listen_for_unlock(signal: ResumeSignal) -> zbus::Result<()> {
        let connection = Connection::system()?;
        let manager = Proxy::new(
            &connection,
            LOGIN_DESTINATION,
            LOGIN_MANAGER_PATH,
            LOGIN_MANAGER_INTERFACE,
        )?;
        let session_path: OwnedObjectPath =
            manager.call("GetSessionByPID", &(std::process::id(),))?;
        let session = Proxy::new(
            &connection,
            LOGIN_DESTINATION,
            session_path,
            LOGIN_SESSION_INTERFACE,
        )?;
        for change in session.receive_property_changed::<bool>("LockedHint") {
            if !change.get()? {
                signal.request();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ResumeSignal;

    #[test]
    fn resume_requests_are_coalesced() {
        let signal = ResumeSignal::new();

        signal.request();
        signal.request();

        assert!(signal.take());
        assert!(!signal.take());
    }

    #[test]
    fn resume_request_can_wait_for_manual_connection_result() {
        let signal = ResumeSignal::new();

        signal.request();
        assert!(signal.is_requested());

        signal.clear();
        assert!(!signal.is_requested());
    }
}
