//! JNI bridge infrastructure: Activity reference, JNIEnv management, class caching.

use std::sync::OnceLock;

use android_activity::AndroidApp;
use jni::objects::{GlobalRef, JObject};
use jni::{JNIEnv, JavaVM};

#[allow(dead_code)]
struct Bridge {
    vm: JavaVM,
    activity: GlobalRef,
    activity_class: GlobalRef,
}

static BRIDGE: OnceLock<Bridge> = OnceLock::new();

pub fn init(android_app: &AndroidApp) {
    let vm_ptr = android_app.vm_as_ptr() as *mut jni::sys::JavaVM;
    let activity_ptr = android_app.activity_as_ptr() as jni::sys::jobject;

    // Create a temporary VM reference to set up global refs, then forget it
    // (android-activity owns the VM lifetime)
    let (activity, activity_class) = {
        let vm = unsafe { JavaVM::from_raw(vm_ptr) }.expect("failed to get JavaVM");
        let env = vm
            .attach_current_thread()
            .expect("failed to attach JNI thread");

        let activity_obj = unsafe { JObject::from_raw(activity_ptr) };
        let activity = env
            .new_global_ref(&activity_obj)
            .expect("failed to create Activity global ref");

        let class = env
            .get_object_class(&activity_obj)
            .expect("failed to get Activity class");
        let activity_class = env
            .new_global_ref(class)
            .expect("failed to create Activity class global ref");

        // Drop env (AttachGuard) before forgetting vm
        drop(env);
        std::mem::forget(vm);

        (activity, activity_class)
    };

    // Create the stored VM reference
    let vm = unsafe { JavaVM::from_raw(vm_ptr) }.expect("failed to get JavaVM");

    BRIDGE
        .set(Bridge {
            vm,
            activity,
            activity_class,
        })
        .ok()
        .expect("rinch-android already initialized");

    // Register native methods for RinchInputConnection.
    // FindClass from native threads uses the system classloader which can't see
    // app classes. We use the Activity's classloader instead.
    with_activity(|env, activity| {
        let class_loader = env
            .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])
            .and_then(|v| v.l())
            .expect("failed to get classloader");

        let class_name = env.new_string("com.rinch.RinchInputConnection").unwrap();
        let ic_class = env
            .call_method(
                &class_loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[jni::objects::JValue::Object(&class_name)],
            )
            .and_then(|v| v.l())
            .expect("failed to load RinchInputConnection class");

        let ic_jclass = jni::objects::JClass::from(ic_class);
        env.register_native_methods(
            ic_jclass,
            &[
                jni::NativeMethod {
                    name: "nativeCommitText".into(),
                    sig: "(Ljava/lang/String;)V".into(),
                    fn_ptr: crate::ime::Java_com_rinch_RinchInputConnection_nativeCommitText
                        as *mut std::ffi::c_void,
                },
                jni::NativeMethod {
                    name: "nativeDeleteSurrounding".into(),
                    sig: "(II)V".into(),
                    fn_ptr: crate::ime::Java_com_rinch_RinchInputConnection_nativeDeleteSurrounding
                        as *mut std::ffi::c_void,
                },
            ],
        )
        .expect("failed to register RinchInputConnection native methods");

        log::info!("RinchInputConnection native methods registered");

        // Register native methods for RinchActivity (callback routing).
        let activity_class_name = env.new_string("com.rinch.RinchActivity").unwrap();
        let act_class = env
            .call_method(
                &class_loader,
                "loadClass",
                "(Ljava/lang/String;)Ljava/lang/Class;",
                &[jni::objects::JValue::Object(&activity_class_name)],
            )
            .and_then(|v| v.l())
            .expect("failed to load RinchActivity class");

        let act_jclass = jni::objects::JClass::from(act_class);
        env.register_native_methods(
            act_jclass,
            &[
                jni::NativeMethod {
                    name: "nativeOnActivityResult".into(),
                    sig: "(IILjava/lang/String;)V".into(),
                    fn_ptr: crate::callback::Java_com_rinch_RinchActivity_nativeOnActivityResult
                        as *mut std::ffi::c_void,
                },
                jni::NativeMethod {
                    name: "nativeOnPermissionsResult".into(),
                    sig: "(IZ)V".into(),
                    fn_ptr: crate::callback::Java_com_rinch_RinchActivity_nativeOnPermissionsResult
                        as *mut std::ffi::c_void,
                },
                jni::NativeMethod {
                    name: "nativeOnSensorChanged".into(),
                    sig: "(I[FJ)V".into(),
                    fn_ptr: crate::sensors::Java_com_rinch_RinchActivity_nativeOnSensorChanged
                        as *mut std::ffi::c_void,
                },
                jni::NativeMethod {
                    name: "nativeOnLocationChanged".into(),
                    sig: "(DDDFFFJLjava/lang/String;)V".into(),
                    fn_ptr: crate::location::Java_com_rinch_RinchActivity_nativeOnLocationChanged
                        as *mut std::ffi::c_void,
                },
                jni::NativeMethod {
                    name: "nativeOnPause".into(),
                    sig: "()V".into(),
                    fn_ptr: crate::lifecycle::Java_com_rinch_RinchActivity_nativeOnPause
                        as *mut std::ffi::c_void,
                },
                jni::NativeMethod {
                    name: "nativeOnResume".into(),
                    sig: "()V".into(),
                    fn_ptr: crate::lifecycle::Java_com_rinch_RinchActivity_nativeOnResume
                        as *mut std::ffi::c_void,
                },
            ],
        )
        .expect("failed to register RinchActivity native methods");

        log::info!("RinchActivity native methods registered");
    });
}

fn bridge() -> &'static Bridge {
    BRIDGE
        .get()
        .expect("rinch-android not initialized — call rinch_android::init() first")
}

pub fn with_jni_env<R>(f: impl FnOnce(&mut JNIEnv) -> R) -> R {
    let b = bridge();
    let mut env =
        b.vm.attach_current_thread()
            .expect("failed to attach JNI thread");
    f(&mut env)
}

pub fn with_activity<R>(f: impl FnOnce(&mut JNIEnv, &JObject) -> R) -> R {
    with_jni_env(|env| f(env, bridge().activity.as_obj()))
}

#[allow(dead_code)]
pub fn activity_class() -> &'static GlobalRef {
    &bridge().activity_class
}

pub fn register_input_connection_natives(env: &mut JNIEnv) {
    let ic_class = env
        .find_class("com/rinch/RinchInputConnection")
        .expect("RinchInputConnection class not found");
    env.register_native_methods(
        ic_class,
        &[
            jni::NativeMethod {
                name: "nativeCommitText".into(),
                sig: "(Ljava/lang/String;)V".into(),
                fn_ptr: crate::ime::Java_com_rinch_RinchInputConnection_nativeCommitText
                    as *mut std::ffi::c_void,
            },
            jni::NativeMethod {
                name: "nativeDeleteSurrounding".into(),
                sig: "(II)V".into(),
                fn_ptr: crate::ime::Java_com_rinch_RinchInputConnection_nativeDeleteSurrounding
                    as *mut std::ffi::c_void,
            },
        ],
    )
    .expect("failed to register RinchInputConnection native methods");
}

#[unsafe(no_mangle)]
pub extern "C" fn Java_com_rinch_RinchActivity_nativeRegisterInputMethods(
    mut env: JNIEnv,
    _class: jni::objects::JClass,
) {
    register_input_connection_natives(&mut env);
}
