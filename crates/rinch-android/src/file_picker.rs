//! Android SAF (Storage Access Framework) file picker.
//!
//! Provides `pick_file` and `save_file` using ACTION_OPEN_DOCUMENT /
//! ACTION_CREATE_DOCUMENT intents, plus `read_content_uri` and
//! `write_content_uri` to move bytes across a `content://` URI via
//! ContentResolver.

use jni::objects::JValue;

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
        if let Err(e) = env.call_method(activity, "openFilePicker", "(I)V", &[JValue::Int(code)]) {
            log::warn!("openFilePicker JNI call failed: {e}");
        }
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
        let jname = match env.new_string(&file_name) {
            Ok(s) => s,
            Err(e) => {
                log::warn!("Failed to create JNI string for file name: {e}");
                return;
            }
        };
        if let Err(e) = env.call_method(
            activity,
            "saveFilePicker",
            "(ILjava/lang/String;)V",
            &[JValue::Int(code), JValue::Object(&jname)],
        ) {
            log::warn!("saveFilePicker JNI call failed: {e}");
        }
    });
}

/// Read all bytes from a `content://` URI via the Java ContentResolver.
/// Returns the file contents or an error description.
pub fn read_content_uri(uri: &str) -> Result<Vec<u8>, String> {
    bridge::with_activity(|env, activity| {
        let juri = env.new_string(uri).map_err(|e| e.to_string())?;
        let result = env
            .call_method(
                activity,
                "readContentUri",
                "(Ljava/lang/String;)[B",
                &[JValue::Object(&juri)],
            )
            .map_err(|e| format!("readContentUri JNI call failed: {e}"))?
            .l()
            .map_err(|e| format!("readContentUri return type error: {e}"))?;

        if result.is_null() {
            return Err("readContentUri returned null (IO error or invalid URI)".into());
        }

        let jbyte_array: jni::objects::JByteArray = result.into();
        let len = env
            .get_array_length(&jbyte_array)
            .map_err(|e| format!("get_array_length failed: {e}"))?;

        let mut buf = vec![0i8; len as usize];
        env.get_byte_array_region(&jbyte_array, 0, &mut buf)
            .map_err(|e| format!("get_byte_array_region failed: {e}"))?;

        // Convert i8 array to u8 array (safe reinterpret)
        let bytes: Vec<u8> = buf.into_iter().map(|b| b as u8).collect();
        Ok(bytes)
    })
}

/// Write bytes to a `content://` URI via the Java ContentResolver — the other
/// half of `read_content_uri`, and the piece `save_file` above needs to be
/// useful on its own: `save_file`'s callback only ever hands back the URI of a
/// document `ACTION_CREATE_DOCUMENT` created empty, and this is what puts the
/// caller's bytes into it. Returns an error description on failure rather than
/// nothing to write to, since a failed save is a failure a caller has to be
/// able to show.
pub fn write_content_uri(uri: &str, bytes: &[u8]) -> Result<(), String> {
    bridge::with_activity(|env, activity| {
        let juri = env.new_string(uri).map_err(|e| e.to_string())?;
        let jbytes = env.byte_array_from_slice(bytes).map_err(|e| e.to_string())?;
        let ok = env
            .call_method(
                activity,
                "writeContentUri",
                "(Ljava/lang/String;[B)Z",
                &[JValue::Object(&juri), JValue::Object(&jbytes)],
            )
            .map_err(|e| format!("writeContentUri JNI call failed: {e}"))?
            .z()
            .map_err(|e| format!("writeContentUri return type error: {e}"))?;

        if ok {
            Ok(())
        } else {
            Err("writeContentUri returned false (IO error or invalid URI)".into())
        }
    })
}
