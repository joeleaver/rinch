//! Android SAF (Storage Access Framework) file picker.
//!
//! Provides `pick_file` and `save_file` using ACTION_OPEN_DOCUMENT /
//! ACTION_CREATE_DOCUMENT intents, plus `read_content_uri` and
//! `write_content_uri` to move bytes across a `content://` URI via
//! ContentResolver.

use jni::objects::JValue;

use crate::jni_exception::{jni_ok, jni_try};
use crate::{bridge, callback};

/// Android RESULT_OK constant.
const RESULT_OK: i32 = -1;

/// Open a SAF file picker. The callback receives `Some(content_uri)` on success
/// or `None` if the user cancelled.
pub fn pick_file(cb: impl FnOnce(Option<String>) + 'static) {
    let code = callback::next_request_code();
    callback::register_activity_callback(code, move |result| {
        cb(if result.result_code == RESULT_OK {
            result.data_uri
        } else {
            None
        })
    });
    bridge::with_activity(|env, activity| {
        jni_ok(env, "openFilePicker", |env| {
            env.call_method(activity, "openFilePicker", "(I)V", &[JValue::Int(code)])
        });
    });
}

/// Open a SAF save-file picker with a suggested file name. The callback receives
/// `Some(content_uri)` on success or `None` if the user cancelled.
pub fn save_file(file_name: &str, cb: impl FnOnce(Option<String>) + 'static) {
    let code = callback::next_request_code();
    callback::register_activity_callback(code, move |result| {
        cb(if result.result_code == RESULT_OK {
            result.data_uri
        } else {
            None
        })
    });
    let file_name = file_name.to_string();
    bridge::with_activity(|env, activity| {
        let Some(jname) = jni_ok(env, "Failed to create JNI string for file name", |env| {
            env.new_string(&file_name)
        }) else {
            return;
        };
        jni_ok(env, "saveFilePicker", |env| {
            env.call_method(
                activity,
                "saveFilePicker",
                "(ILjava/lang/String;)V",
                &[JValue::Int(code), JValue::Object(&jname)],
            )
        });
    });
}

/// Read all bytes from a `content://` URI via the Java ContentResolver.
/// Returns the file contents or an error description.
pub fn read_content_uri(uri: &str) -> Result<Vec<u8>, String> {
    bridge::with_activity(|env, activity| {
        let juri = jni_try(env, "readContentUri: new_string", |env| env.new_string(uri))?;
        let result = jni_try(env, "readContentUri JNI call failed", |env| {
            env.call_method(
                activity,
                "readContentUri",
                "(Ljava/lang/String;)[B",
                &[JValue::Object(&juri)],
            )?
            .l()
        })?;

        if result.is_null() {
            // A failure to open or read throws, and `jni_try` names it (#1215);
            // null is only a provider that handed back no stream.
            return Err("readContentUri: the provider opened no stream".into());
        }

        let jbyte_array: jni::objects::JByteArray = result.into();
        let len = jni_try(env, "get_array_length failed", |env| {
            env.get_array_length(&jbyte_array)
        })?;

        let mut buf = vec![0i8; len as usize];
        jni_try(env, "get_byte_array_region failed", |env| {
            env.get_byte_array_region(&jbyte_array, 0, &mut buf)
        })?;

        // Convert i8 array to u8 array (safe reinterpret)
        let bytes: Vec<u8> = buf.into_iter().map(|b| b as u8).collect();
        Ok(bytes)
    })
}

/// Write bytes to a `content://` URI via the Java ContentResolver — the other
/// half of [`read_content_uri`], and the piece [`save_file`] needs to be useful
/// on its own: `save_file`'s callback hands back a URI, and this is what puts
/// the caller's bytes into it. Returns an error description on failure, since a
/// failed save is a failure a caller has to be able to show.
///
/// **Replaces the document's contents.** The write opens the URI in mode `"wt"`
/// — truncating — so a shorter write does not leave the tail of whatever was
/// there before. That is not the platform default: `openOutputStream(uri)` uses
/// `"w"`, whose truncation the Android javadoc explicitly leaves to each
/// provider ("`w` may or may not truncate"), and the framework's own
/// `translateModeStringToPosix` maps it to `O_WRONLY | O_CREAT` with no
/// `O_TRUNC`. Saving 6KB over an existing 10KB document would keep the last 4KB
/// and still report success.
///
/// Do not assume the target starts empty just because `ACTION_CREATE_DOCUMENT`
/// produced it: a provider may return an existing document when the user picks
/// a name that is already taken, an app may keep the URI and save to it again,
/// and this function accepts any `content://` URI the caller holds.
///
/// There is no partial-write contract. On `Err` the document may hold anything
/// from its previous contents to a truncated prefix — the underlying stream is
/// closed either way, but how much reached the provider is not knowable from
/// here.
pub fn write_content_uri(uri: &str, bytes: &[u8]) -> Result<(), String> {
    bridge::with_activity(|env, activity| {
        let juri = jni_try(env, "writeContentUri: new_string", |env| {
            env.new_string(uri)
        })?;
        let jbytes = jni_try(env, "writeContentUri: byte array", |env| {
            env.byte_array_from_slice(bytes)
        })?;
        let ok = jni_try(env, "writeContentUri JNI call failed", |env| {
            env.call_method(
                activity,
                "writeContentUri",
                "(Ljava/lang/String;[B)Z",
                &[JValue::Object(&juri), JValue::Object(&jbytes)],
            )?
            .z()
        })?;

        if ok {
            Ok(())
        } else {
            // As for the reader: a failure throws and is named above (#1215).
            Err("writeContentUri: the provider opened no stream".into())
        }
    })
}
