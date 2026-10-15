//! Windows: WinRT toast notifications (`Windows.UI.Notifications`), created
//! and shown on a thread of their own (a multithreaded apartment), so no
//! COM call ever runs on the main thread.
//!
//! An unpackaged app's toasts are attributed to an AppUserModelID. Windows
//! shows them only for an AUMID it knows: one carried by a Start-menu
//! shortcut, or one registered under
//! `HKCU\Software\Classes\AppUserModelId\<AUMID>` (what the Windows App SDK
//! does for unpackaged apps). rinch writes the registry key — `DisplayName`
//! only — on the first notification, unless the key exists already or the app
//! said not to ([`crate::notification::set_register_app`]).
//!
//! Click and dismissal come from the toast's own `Activated` / `Dismissed` /
//! `Failed` events, which Windows raises while the process is running. A
//! toast clicked in the Action Center after the app exited would need a COM
//! activator (`CustomActivator`) to relaunch it; rinch registers none.

use std::collections::HashMap;
use std::sync::mpsc;

use ::windows::Data::Xml::Dom::XmlDocument;
use ::windows::Foundation::TypedEventHandler;
use ::windows::UI::Notifications::{
    ToastDismissalReason, ToastDismissedEventArgs, ToastFailedEventArgs, ToastNotification,
    ToastNotificationManager, ToastNotifier,
};
use ::windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
use ::windows::core::{HSTRING, IInspectable};

use super::toast_xml::toast_xml;
use crate::notification::{Backend, DismissReason, Event, NotificationError, Request, Sink};

enum Cmd {
    Show(Request),
    Close(u64),
    /// A toast reported its end; forget it.
    Done(u64),
}

pub(crate) struct ToastBackend {
    tx: mpsc::Sender<Cmd>,
}

impl ToastBackend {
    pub(crate) fn start(sink: Sink) -> Result<Self, NotificationError> {
        let (tx, rx) = mpsc::channel::<Cmd>();
        let done = tx.clone();
        std::thread::Builder::new()
            .name("rinch-notifications".into())
            .spawn(move || run(rx, done, sink))
            .map_err(|e| NotificationError::Platform(e.to_string()))?;
        Ok(ToastBackend { tx })
    }
}

impl Backend for ToastBackend {
    fn show(&self, req: Request) -> Result<(), NotificationError> {
        self.tx
            .send(Cmd::Show(req))
            .map_err(|_| NotificationError::Platform("the notification thread has stopped".into()))
    }

    fn close(&self, id: u64) {
        let _ = self.tx.send(Cmd::Close(id));
    }
}

fn platform(e: ::windows::core::Error) -> NotificationError {
    NotificationError::Platform(e.to_string())
}

fn run(rx: mpsc::Receiver<Cmd>, done: mpsc::Sender<Cmd>, sink: Sink) {
    // SAFETY: called once, first, on this thread, which makes no other COM
    // call before it.
    if let Err(e) = unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
        tracing::warn!("notifications: RoInitialize failed: {e}");
    }
    let mut notifier: Option<(String, ToastNotifier)> = None;
    let mut toasts: HashMap<u64, ToastNotification> = HashMap::new();
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Cmd::Show(req) => {
                let id = req.id;
                match show(&req, &mut notifier, &done, &sink) {
                    Ok(toast) => {
                        toasts.insert(id, toast);
                    }
                    Err(e) => sink(id, Event::Failed(e)),
                }
            }
            Cmd::Close(id) => {
                if let (Some(toast), Some((_, n))) = (toasts.get(&id), &notifier) {
                    // Raises `Dismissed` with `ApplicationHidden`.
                    let _ = n.Hide(toast);
                }
            }
            Cmd::Done(id) => {
                toasts.remove(&id);
            }
        }
    }
}

fn show(
    req: &Request,
    notifier: &mut Option<(String, ToastNotifier)>,
    done: &mpsc::Sender<Cmd>,
    sink: &Sink,
) -> Result<ToastNotification, NotificationError> {
    if notifier
        .as_ref()
        .is_none_or(|(aumid, _)| *aumid != req.app_id)
    {
        if crate::notification::may_register_app() {
            if let Err(e) = register_aumid(&req.app_id, &req.app_name) {
                tracing::warn!(
                    "notifications: could not register AppUserModelID {:?}: {e}",
                    req.app_id
                );
            }
        }
        let n = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(
            req.app_id.as_str(),
        ))
        .map_err(platform)?;
        *notifier = Some((req.app_id.clone(), n));
    }
    let Some((_, n)) = notifier.as_ref() else {
        unreachable!("set above")
    };

    let doc = XmlDocument::new().map_err(platform)?;
    doc.LoadXml(&HSTRING::from(toast_xml(req)))
        .map_err(platform)?;
    let toast = ToastNotification::CreateToastNotification(&doc).map_err(platform)?;

    let id = req.id;
    {
        let (sink, done) = (sink.clone(), done.clone());
        toast
            .Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
                move |_, _| {
                    sink(id, Event::Clicked);
                    let _ = done.send(Cmd::Done(id));
                    Ok(())
                },
            ))
            .map_err(platform)?;
    }
    {
        let (sink, done) = (sink.clone(), done.clone());
        toast
            .Dismissed(&TypedEventHandler::<
                ToastNotification,
                ToastDismissedEventArgs,
            >::new(move |_, args| {
                let reason = args
                    .as_ref()
                    .and_then(|a| a.Reason().ok())
                    .map_or(DismissReason::Unknown, dismiss_reason);
                sink(id, Event::Dismissed(reason));
                let _ = done.send(Cmd::Done(id));
                Ok(())
            }))
            .map_err(platform)?;
    }
    {
        let (sink, done) = (sink.clone(), done.clone());
        toast
            .Failed(
                &TypedEventHandler::<ToastNotification, ToastFailedEventArgs>::new(
                    move |_, args| {
                        let code = args
                            .as_ref()
                            .and_then(|a| a.ErrorCode().ok())
                            .map_or_else(|| "unknown error".to_string(), |c| c.message());
                        sink(
                            id,
                            Event::Failed(NotificationError::Platform(format!(
                                "Windows did not show the toast: {code}"
                            ))),
                        );
                        let _ = done.send(Cmd::Done(id));
                        Ok(())
                    },
                ),
            )
            .map_err(platform)?;
    }
    n.Show(&toast).map_err(platform)?;
    Ok(toast)
}

fn dismiss_reason(r: ToastDismissalReason) -> DismissReason {
    match r {
        ToastDismissalReason::UserCanceled => DismissReason::Dismissed,
        ToastDismissalReason::ApplicationHidden => DismissReason::Closed,
        ToastDismissalReason::TimedOut => DismissReason::Expired,
        _ => DismissReason::Unknown,
    }
}

/// Write `HKCU\Software\Classes\AppUserModelId\<aumid>` with a `DisplayName`,
/// unless the key exists.
fn register_aumid(aumid: &str, display_name: &str) -> Result<(), String> {
    use ::windows::Win32::Foundation::ERROR_SUCCESS;
    use ::windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegOpenKeyExW, RegSetValueExW,
    };

    let path = HSTRING::from(format!("Software\\Classes\\AppUserModelId\\{aumid}"));
    let mut key = HKEY::default();
    // SAFETY: valid HKEY constant, NUL-terminated path, out-pointer to a
    // local; the key is closed below.
    let open = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, &path, None, KEY_READ, &mut key) };
    if open == ERROR_SUCCESS {
        // SAFETY: the key was opened above.
        unsafe {
            let _ = RegCloseKey(key);
        }
        return Ok(());
    }
    // SAFETY: as above; `key` receives the created key.
    let created = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            &path,
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    if created != ERROR_SUCCESS {
        return Err(format!("RegCreateKeyExW: {created:?}"));
    }
    let value: Vec<u8> = display_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect();
    // SAFETY: `key` is open for writing; `value` is a NUL-terminated UTF-16
    // string as bytes.
    let set = unsafe {
        RegSetValueExW(
            key,
            &HSTRING::from("DisplayName"),
            None,
            REG_SZ,
            Some(&value),
        )
    };
    // SAFETY: the key was created above.
    unsafe {
        let _ = RegCloseKey(key);
    }
    if set != ERROR_SUCCESS {
        return Err(format!("RegSetValueExW: {set:?}"));
    }
    Ok(())
}
