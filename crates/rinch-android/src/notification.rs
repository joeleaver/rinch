//! Android notifications via NotificationManager.
//!
//! Creates a notification channel on first use and posts notifications
//! with a title, body text, and optional icon priority.

use jni::objects::JValue;

use crate::bridge;

/// Notification priority level.
#[derive(Clone, Copy, Debug, Default)]
pub enum Priority {
    Low,
    #[default]
    Default,
    High,
}

impl Priority {
    fn to_importance(self) -> i32 {
        match self {
            Priority::Low => 2,     // IMPORTANCE_LOW
            Priority::Default => 4, // IMPORTANCE_HIGH (shows heads-up)
            Priority::High => 4,    // IMPORTANCE_HIGH
        }
    }
}

/// Show a notification with the given title and body text.
/// Requests `POST_NOTIFICATIONS` permission on Android 13+ if needed.
pub fn show(title: &str, body: &str) {
    show_with_priority(title, body, Priority::Default);
}

/// Show a notification with a specific priority level.
/// Requests `POST_NOTIFICATIONS` permission on Android 13+ if needed.
pub fn show_with_priority(title: &str, body: &str, priority: Priority) {
    let perm = "android.permission.POST_NOTIFICATIONS";
    if !crate::permissions::has_permission(perm) {
        let title = title.to_string();
        let body = body.to_string();
        crate::permissions::request_permission(perm, move |granted| {
            if granted {
                post_notification(&title, &body, priority);
            }
        });
    } else {
        post_notification(title, body, priority);
    }
}

fn post_notification(title: &str, body: &str, priority: Priority) {
    let title = title.to_string();
    let body = body.to_string();
    let importance = priority.to_importance();
    bridge::with_activity(|env, activity| {
        let jtitle = match env.new_string(&title) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("notification: failed to create title string: {e}");
                return;
            }
        };
        let jbody = match env.new_string(&body) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("notification: failed to create body string: {e}");
                return;
            }
        };
        if let Err(e) = env.call_method(
            activity,
            "showNotification",
            "(Ljava/lang/String;Ljava/lang/String;I)V",
            &[
                JValue::Object(&jtitle),
                JValue::Object(&jbody),
                JValue::Int(importance),
            ],
        ) {
            log::warn!("showNotification JNI call failed: {e}");
        }
    });
}
