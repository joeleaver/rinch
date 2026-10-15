//! macOS: the UserNotifications framework (`UNUserNotificationCenter`),
//! spoken through `objc2` messages.
//!
//! Every call here is asynchronous on the system's side — authorisation and
//! delivery answer through completion handlers on a queue of the system's —
//! so `show` runs on the calling thread and never waits.
//!
//! The framework works only for a process with a bundle identifier (an `.app`
//! bundle); unbundled, `currentNotificationCenter` raises an Objective-C
//! exception. So the backend refuses to start without one and `show`
//! answers [`NotificationError::Unsupported`].
//!
//! A click arrives at the center's delegate
//! (`userNotificationCenter:didReceiveNotificationResponse:…`). A dismissal
//! is reported only for a category with `customDismissAction`; rinch
//! registers none, so `on_dismiss` runs on macOS only for
//! [`NotificationHandle::close`](crate::notification::NotificationHandle::close).
//!
//! The delegate is installed on the first `show`. So a click that launches
//! (or relaunches) the app reaches no callback, and an app or crate that
//! sets its own `UNUserNotificationCenter` delegate replaces rinch's (or is
//! replaced by it): there is one delegate per process. No sound is asked
//! for, as on the other platforms.

use std::ffi::{CStr, CString, c_char};
use std::sync::{Once, OnceLock};

use block2::{Block, RcBlock};
use objc2::rc::{Retained, autoreleasepool};
use objc2::runtime::{AnyObject, Bool, NSObject};
use objc2::{ClassType, class, define_class, msg_send};

use crate::notification::{Backend, DismissReason, Event, NotificationError, Request, Sink};

#[link(name = "UserNotifications", kind = "framework")]
unsafe extern "C" {}

/// `UNAuthorizationOptionSound | UNAuthorizationOptionAlert`.
const AUTH_OPTIONS: usize = (1 << 1) | (1 << 2);
/// `UNNotificationPresentationOptionSound | …List | …Banner`: show it even
/// while the app is frontmost.
const PRESENT_OPTIONS: usize = (1 << 1) | (1 << 3) | (1 << 4);
const DEFAULT_ACTION: &str = "com.apple.UNNotificationDefaultActionIdentifier";
const DISMISS_ACTION: &str = "com.apple.UNNotificationDismissActionIdentifier";

static SINK: OnceLock<Sink> = OnceLock::new();

pub(crate) struct UserNotificationsBackend;

impl UserNotificationsBackend {
    pub(crate) fn start(sink: Sink) -> Result<Self, NotificationError> {
        if bundle_identifier().is_none() {
            return Err(NotificationError::Unsupported(
                "macOS shows notifications only for an app bundle with a bundle identifier \
                 (CFBundleIdentifier); this executable is not running from one"
                    .into(),
            ));
        }
        let _ = SINK.set(sink);
        Ok(UserNotificationsBackend)
    }
}

fn sink(id: u64, ev: Event) {
    if let Some(s) = SINK.get() {
        s(id, ev);
    }
}

fn bundle_identifier() -> Option<String> {
    autoreleasepool(|_| {
        // SAFETY: `mainBundle` and `bundleIdentifier` are plain getters on
        // NSBundle, answering an object or nil.
        unsafe {
            let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
            if bundle.is_null() {
                return None;
            }
            let ident: *mut AnyObject = msg_send![bundle, bundleIdentifier];
            to_string(ident)
        }
    })
}

/// An autoreleased `NSString`.
unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = CString::new(s.replace('\0', "")).unwrap_or_default();
    // SAFETY: a NUL-terminated UTF-8 string.
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}

unsafe fn to_string(ns: *mut AnyObject) -> Option<String> {
    if ns.is_null() {
        return None;
    }
    // SAFETY: `ns` is an NSString; `UTF8String` lives as long as it does.
    unsafe {
        let p: *const c_char = msg_send![ns, UTF8String];
        (!p.is_null()).then(|| CStr::from_ptr(p).to_string_lossy().into_owned())
    }
}

unsafe fn error_description(error: *mut AnyObject) -> String {
    // SAFETY: `error` is an NSError or nil.
    unsafe {
        if error.is_null() {
            return "unknown error".into();
        }
        let d: *mut AnyObject = msg_send![error, localizedDescription];
        to_string(d).unwrap_or_else(|| "unknown error".into())
    }
}

fn prefix() -> String {
    format!("rinch-{}-", std::process::id())
}

fn identifier(id: u64) -> String {
    format!("{}{id}", prefix())
}

/// The rinch id a request identifier names, if this process made it.
fn parse_identifier(s: &str) -> Option<u64> {
    s.strip_prefix(&prefix())?.parse().ok()
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and this class
    // implements no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "RinchUserNotificationsDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
        fn did_receive(
            &self,
            _center: *mut AnyObject,
            response: *mut AnyObject,
            handler: &Block<dyn Fn()>,
        ) {
            // SAFETY: `response` is the framework's UNNotificationResponse.
            unsafe {
                let action: *mut AnyObject = msg_send![response, actionIdentifier];
                let notification: *mut AnyObject = msg_send![response, notification];
                let request: *mut AnyObject = msg_send![notification, request];
                let ident: *mut AnyObject = msg_send![request, identifier];
                if let (Some(action), Some(id)) = (
                    to_string(action),
                    to_string(ident).as_deref().and_then(parse_identifier),
                ) {
                    if action == DEFAULT_ACTION {
                        sink(id, Event::Clicked);
                    } else if action == DISMISS_ACTION {
                        sink(id, Event::Dismissed(DismissReason::Dismissed));
                    }
                }
            }
            handler.call(());
        }

        #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
        fn will_present(
            &self,
            _center: *mut AnyObject,
            _notification: *mut AnyObject,
            handler: &Block<dyn Fn(usize)>,
        ) {
            handler.call((PRESENT_OPTIONS,));
        }
    }
);

fn center() -> *mut AnyObject {
    static DELEGATE: Once = Once::new();
    // SAFETY: the process has a bundle identifier (checked at start), so
    // `currentNotificationCenter` does not raise.
    let center: *mut AnyObject =
        unsafe { msg_send![class!(UNUserNotificationCenter), currentNotificationCenter] };
    DELEGATE.call_once(|| {
        // SAFETY: a plain NSObject subclass; the center holds its delegate
        // weakly, so the instance is leaked to live for the process.
        unsafe {
            let delegate: Retained<Delegate> = msg_send![Delegate::class(), new];
            let delegate = Retained::into_raw(delegate);
            let _: () = msg_send![center, setDelegate: delegate as *mut AnyObject];
        }
    });
    center
}

impl Backend for UserNotificationsBackend {
    fn show(&self, req: Request) -> Result<(), NotificationError> {
        autoreleasepool(|_| {
            let center = center();
            let id = req.id;
            let block = RcBlock::new(move |granted: Bool, error: *mut AnyObject| {
                if !granted.as_bool() {
                    let err = if error.is_null() {
                        NotificationError::PermissionDenied
                    } else {
                        // SAFETY: an NSError from the framework.
                        let code: isize = unsafe { msg_send![error, code] };
                        // UNErrorCodeNotificationsNotAllowed
                        if code == 1 {
                            NotificationError::PermissionDenied
                        } else {
                            NotificationError::Platform(unsafe { error_description(error) })
                        }
                    };
                    sink(id, Event::Failed(err));
                    return;
                }
                autoreleasepool(|_| add_request(&req));
            });
            // SAFETY: the center is live; the block is copied by the
            // framework before this returns.
            unsafe {
                let _: () = msg_send![
                    center,
                    requestAuthorizationWithOptions: AUTH_OPTIONS,
                    completionHandler: &*block
                ];
            }
        });
        Ok(())
    }

    fn close(&self, id: u64) {
        autoreleasepool(|_| {
            let center = center();
            // SAFETY: an array of one NSString identifier.
            unsafe {
                let ident = ns_string(&identifier(id));
                let ids: *mut AnyObject = msg_send![class!(NSArray), arrayWithObject: ident];
                let _: () = msg_send![center, removeDeliveredNotificationsWithIdentifiers: ids];
                let _: () =
                    msg_send![center, removePendingNotificationRequestsWithIdentifiers: ids];
            }
        });
        sink(id, Event::Dismissed(DismissReason::Closed));
    }
}

fn add_request(req: &Request) {
    let id = req.id;
    // SAFETY: plain UserNotifications / Foundation calls with live objects;
    // objects made with `new` are released through `Retained`.
    unsafe {
        let content: Retained<AnyObject> = msg_send![class!(UNMutableNotificationContent), new];
        let _: () = msg_send![&*content, setTitle: ns_string(&req.title)];
        if !req.body.is_empty() {
            let _: () = msg_send![&*content, setBody: ns_string(&req.body)];
        }
        if let Some(icon) = &req.icon {
            // The system moves an attachment's file into its own store, so
            // it gets a copy of its own.
            let copy = icon.with_file_name(format!("{}.png", identifier(id)));
            if std::fs::copy(icon, &copy).is_ok() {
                let path = ns_string(&copy.to_string_lossy());
                let url: *mut AnyObject = msg_send![class!(NSURL), fileURLWithPath: path];
                let mut error: *mut AnyObject = std::ptr::null_mut();
                let attachment: *mut AnyObject = msg_send![
                    class!(UNNotificationAttachment),
                    attachmentWithIdentifier: ns_string("icon"),
                    URL: url,
                    options: std::ptr::null_mut::<AnyObject>(),
                    error: &mut error
                ];
                if !attachment.is_null() {
                    let list: *mut AnyObject =
                        msg_send![class!(NSArray), arrayWithObject: attachment];
                    let _: () = msg_send![&*content, setAttachments: list];
                } else {
                    tracing::warn!(
                        "notification {id}: the icon was not attached: {}",
                        error_description(error)
                    );
                }
            }
        }
        let request: *mut AnyObject = msg_send![
            class!(UNNotificationRequest),
            requestWithIdentifier: ns_string(&identifier(id)),
            content: &*content,
            trigger: std::ptr::null_mut::<AnyObject>()
        ];
        let done = RcBlock::new(move |error: *mut AnyObject| {
            if !error.is_null() {
                let msg = error_description(error);
                sink(id, Event::Failed(NotificationError::Platform(msg)));
            }
        });
        let center = center();
        let _: () =
            msg_send![center, addNotificationRequest: request, withCompletionHandler: &*done];
    }
}
