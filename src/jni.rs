// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2025 rhythmcache
// https://github.com/rhythmcache/payload-dumper-rust

//! JNI wrapper for payload dumper operations
//!
//! this code provides jni bindings for
//! payload dumper

use crate::extractor::local::{
    ExtractionProgress, ExtractionStatus, ProgressCallback, extract_partition,
    extract_partition_zip, list_partitions, list_partitions_zip,
};
use crate::extractor::remote::{
    extract_partition_remote_bin, extract_partition_remote_zip, list_partitions_remote_bin,
    list_partitions_remote_zip,
};
use jni::JNIEnv;
use jni::objects::{JClass, JObject, JString, JValue};
use jni::sys::jstring;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;

/* Helper Functions */

/// convert Java string to Rust String
fn jstring_to_string(env: &mut JNIEnv, jstr: &JString) -> Result<String, String> {
    env.get_string(jstr)
        .map(|s| s.into())
        .map_err(|e| format!("Failed to convert Java string: {}", e))
}

/// convert Rust String to Java string (returns owned jString to avoid lifetime issues)
fn string_to_jstring_owned(env: &mut JNIEnv, s: String) -> Result<jstring, String> {
    env.new_string(s)
        .map(|jstr| jstr.into_raw())
        .map_err(|e| format!("Failed to create Java string: {}", e))
}

fn call_java_progress_callback(
    env: &mut JNIEnv,
    callback: &JObject,
    progress: ExtractionProgress,
) -> Result<bool, String> {
    // push a local frame to automatically clean up local references
    env.push_local_frame(16)
        .map_err(|e| format!("Failed to push local frame: {}", e))?;

    let result = (|| {
        let partition_name = env
            .new_string(&progress.partition_name)
            .map_err(|e| format!("Failed to create partition name string: {}", e))?;

        // check for pending exception after new_string
        if env
            .exception_check()
            .map_err(|e| format!("Failed to check exception: {}", e))?
        {
            let _ = env.exception_describe();
            env.exception_clear()
                .map_err(|e| format!("Failed to clear exception: {}", e))?;
            return Err("Exception occurred while creating partition name".to_string());
        }

        let status_value = match progress.status {
            ExtractionStatus::Started => 0,
            ExtractionStatus::InProgress => 1,
            ExtractionStatus::Completed => 2,
            ExtractionStatus::Warning { .. } => 3,
        };

        let (warning_op_index, warning_message) = match progress.status {
            ExtractionStatus::Warning {
                operation_index,
                message,
            } => {
                let msg = env
                    .new_string(&message)
                    .map_err(|e| format!("Failed to create warning message: {}", e))?;

                if env
                    .exception_check()
                    .map_err(|e| format!("Failed to check exception: {}", e))?
                {
                    let _ = env.exception_describe();
                    env.exception_clear()
                        .map_err(|e| format!("Failed to clear exception: {}", e))?;
                    return Err("Exception occurred while creating warning message".to_string());
                }

                (operation_index as i32, msg)
            }
            _ => {
                let empty = env
                    .new_string("")
                    .map_err(|e| format!("Failed to create empty string: {}", e))?;

                if env
                    .exception_check()
                    .map_err(|e| format!("Failed to check exception: {}", e))?
                {
                    let _ = env.exception_describe();
                    env.exception_clear()
                        .map_err(|e| format!("Failed to clear exception: {}", e))?;
                    return Err("Exception occurred while creating empty string".to_string());
                }

                (0, empty)
            }
        };

        let call_result = env
            .call_method(
                callback,
                "onProgress",
                "(Ljava/lang/String;JJDIILjava/lang/String;)Z",
                &[
                    JValue::Object(&partition_name),
                    JValue::Long(progress.current_operation as i64),
                    JValue::Long(progress.total_operations as i64),
                    JValue::Double(progress.percentage),
                    JValue::Int(status_value),
                    JValue::Int(warning_op_index),
                    JValue::Object(&warning_message),
                ],
            )
            .map_err(|e| format!("Failed to call progress callback: {}", e))?;

        // check for exception after method call
        if env
            .exception_check()
            .map_err(|e| format!("Failed to check exception: {}", e))?
        {
            let _ = env.exception_describe();
            env.exception_clear()
                .map_err(|e| format!("Failed to clear exception: {}", e))?;
            return Err("Exception occurred in Java callback".to_string());
        }

        let should_continue = call_result
            .z()
            .map_err(|e| format!("Failed to extract boolean from callback result: {}", e))?;

        Ok(should_continue)
    })();

    let null_obj = JObject::null();
    let _ = unsafe { env.pop_local_frame(&null_obj) };

    result
}

/* Local Operations -> List Partitions */

/// list partitions in a local payload.bin file
///
/// java signature:
/// public static native String listPartitions(String payloadPath);
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rhythmcache_payloaddumper_PayloadDumper_listPartitions(
    mut env: JNIEnv,
    _class: JClass,
    payload_path: JString,
) -> jstring {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let path = jstring_to_string(&mut env, &payload_path)?;
        list_partitions(path).map_err(|e| format!("Failed to list partitions: {}", e))
    }));

    match result {
        Ok(Ok(json)) => match string_to_jstring_owned(&mut env, json) {
            Ok(jstr) => jstr,
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e);
                JObject::null().into_raw()
            }
        },
        Ok(Err(e)) => {
            let _ = env.throw_new("java/lang/RuntimeException", e);
            JObject::null().into_raw()
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                "Panic occurred in native code",
            );
            JObject::null().into_raw()
        }
    }
}

/// list partitions in a local ZIP file containing payload.bin
///
/// java signature:
/// public static native String listPartitionsZip(String zipPath);
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rhythmcache_payloaddumper_PayloadDumper_listPartitionsZip(
    mut env: JNIEnv,
    _class: JClass,
    zip_path: JString,
) -> jstring {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let path = jstring_to_string(&mut env, &zip_path)?;
        list_partitions_zip(path).map_err(|e| format!("Failed to list partitions: {}", e))
    }));

    match result {
        Ok(Ok(json)) => match string_to_jstring_owned(&mut env, json) {
            Ok(jstr) => jstr,
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e);
                JObject::null().into_raw()
            }
        },
        Ok(Err(e)) => {
            let _ = env.throw_new("java/lang/RuntimeException", e);
            JObject::null().into_raw()
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                "Panic occurred in native code",
            );
            JObject::null().into_raw()
        }
    }
}

/* local Operations - Extract Partition */

/// extract a partition from a local payload.bin file
///
/// java signature:
/// public static native void extractPartition(
///     String payloadPath,
///     String partitionName,
///     String outputPath,
///     ProgressCallback callback
/// );
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rhythmcache_payloaddumper_PayloadDumper_extractPartition(
    mut env: JNIEnv,
    _class: JClass,
    payload_path: JString,
    partition_name: JString,
    output_path: JString,
    callback: JObject,
) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let payload = jstring_to_string(&mut env, &payload_path)?;
        let partition = jstring_to_string(&mut env, &partition_name)?;
        let output = jstring_to_string(&mut env, &output_path)?;

        let progress_callback: Option<ProgressCallback> = if !callback.is_null() {
            // create a global reference to the callback object
            // GlobalRef is thread-safe, no need for Mutex wrapper
            let callback_ref = env
                .new_global_ref(&callback)
                .map_err(|e| format!("Failed to create global ref: {}", e))?;

            // we need to get a JavaVM reference that we can use across threads
            let jvm = env
                .get_java_vm()
                .map_err(|e| format!("Failed to get JavaVM: {}", e))?;

            // Arc<GlobalRef> is sufficient - no Mutex needed
            let callback_ref = Arc::new(callback_ref);

            Some(Box::new(move |progress: ExtractionProgress| -> bool {
                // attach to the current thread
                let mut env = match jvm.attach_current_thread() {
                    Ok(env) => env,
                    Err(_) => return false,
                };

                match call_java_progress_callback(&mut env, callback_ref.as_obj(), progress) {
                    Ok(should_continue) => should_continue,
                    Err(_) => false,
                }
            }))
        } else {
            None
        };

        extract_partition(payload, &partition, output, progress_callback)
            .map_err(|e| format!("Failed to extract partition: {}", e))
    }));

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = env.throw_new("java/lang/RuntimeException", e);
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                "Panic occurred in native code",
            );
        }
    }
}

/// extract a partition from a local ZIP file containing payload.bin
///
/// java signature:
/// public static native void extractPartitionZip(
///     String zipPath,
///     String partitionName,
///     String outputPath,
///     ProgressCallback callback
/// );
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rhythmcache_payloaddumper_PayloadDumper_extractPartitionZip(
    mut env: JNIEnv,
    _class: JClass,
    zip_path: JString,
    partition_name: JString,
    output_path: JString,
    callback: JObject,
) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let zip = jstring_to_string(&mut env, &zip_path)?;
        let partition = jstring_to_string(&mut env, &partition_name)?;
        let output = jstring_to_string(&mut env, &output_path)?;

        let progress_callback: Option<ProgressCallback> = if !callback.is_null() {
            let callback_ref = env
                .new_global_ref(&callback)
                .map_err(|e| format!("Failed to create global ref: {}", e))?;

            let jvm = env
                .get_java_vm()
                .map_err(|e| format!("Failed to get JavaVM: {}", e))?;

            let callback_ref = Arc::new(callback_ref);

            Some(Box::new(move |progress: ExtractionProgress| -> bool {
                let mut env = match jvm.attach_current_thread() {
                    Ok(env) => env,
                    Err(_) => return false,
                };

                match call_java_progress_callback(&mut env, callback_ref.as_obj(), progress) {
                    Ok(should_continue) => should_continue,
                    Err(_) => false,
                }
            }))
        } else {
            None
        };

        extract_partition_zip(zip, &partition, output, progress_callback)
            .map_err(|e| format!("Failed to extract partition: {}", e))
    }));

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = env.throw_new("java/lang/RuntimeException", e);
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                "Panic occurred in native code",
            );
        }
    }
}

/* Remote Operations - List Partitions */

/// list partitions in a remote ZIP file containing payload.bin
///
/// java signature:
/// public static native String listPartitionsRemoteZip(String url, String userAgent);
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rhythmcache_payloaddumper_PayloadDumper_listPartitionsRemoteZip(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
    user_agent: JString,
) -> jstring {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<String, String> {
        let url_str = jstring_to_string(&mut env, &url)?;
        let user_agent_str = if !user_agent.is_null() {
            Some(jstring_to_string(&mut env, &user_agent)?)
        } else {
            None
        };

        let result = list_partitions_remote_zip(url_str, user_agent_str.as_deref())
            .map_err(|e| format!("Failed to list remote partitions: {}", e))?;

        Ok(result.json)
    }));

    match result {
        Ok(Ok(json)) => match string_to_jstring_owned(&mut env, json) {
            Ok(jstr) => jstr,
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e);
                JObject::null().into_raw()
            }
        },
        Ok(Err(e)) => {
            let _ = env.throw_new("java/lang/RuntimeException", e);
            JObject::null().into_raw()
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                "Panic occurred in native code",
            );
            JObject::null().into_raw()
        }
    }
}

/// list partitions in a remote payload.bin file (not in ZIP)
///
/// java signature:
/// public static native String listPartitionsRemoteBin(String url, String userAgent);
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rhythmcache_payloaddumper_PayloadDumper_listPartitionsRemoteBin(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
    user_agent: JString,
) -> jstring {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| -> Result<String, String> {
        let url_str = jstring_to_string(&mut env, &url)?;
        let user_agent_str = if !user_agent.is_null() {
            Some(jstring_to_string(&mut env, &user_agent)?)
        } else {
            None
        };

        let result = list_partitions_remote_bin(url_str, user_agent_str.as_deref())
            .map_err(|e| format!("Failed to list remote partitions: {}", e))?;

        Ok(result.json)
    }));

    match result {
        Ok(Ok(json)) => match string_to_jstring_owned(&mut env, json) {
            Ok(jstr) => jstr,
            Err(e) => {
                let _ = env.throw_new("java/lang/RuntimeException", e);
                JObject::null().into_raw()
            }
        },
        Ok(Err(e)) => {
            let _ = env.throw_new("java/lang/RuntimeException", e);
            JObject::null().into_raw()
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                "Panic occurred in native code",
            );
            JObject::null().into_raw()
        }
    }
}

/* Remote Operations - Extract Partition */

/// extract a partition from a remote ZIP file containing payload.bin
///
/// java signature:
/// public static native void extractPartitionRemoteZip(
///     String url,
///     String partitionName,
///     String outputPath,
///     String userAgent,
///     ProgressCallback callback
/// );
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rhythmcache_payloaddumper_PayloadDumper_extractPartitionRemoteZip(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
    partition_name: JString,
    output_path: JString,
    user_agent: JString,
    callback: JObject,
) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let url_str = jstring_to_string(&mut env, &url)?;
        let partition = jstring_to_string(&mut env, &partition_name)?;
        let output = jstring_to_string(&mut env, &output_path)?;
        let user_agent_str = if !user_agent.is_null() {
            Some(jstring_to_string(&mut env, &user_agent)?)
        } else {
            None
        };

        let progress_callback: Option<ProgressCallback> = if !callback.is_null() {
            let callback_ref = env
                .new_global_ref(&callback)
                .map_err(|e| format!("Failed to create global ref: {}", e))?;

            let jvm = env
                .get_java_vm()
                .map_err(|e| format!("Failed to get JavaVM: {}", e))?;

            let callback_ref = Arc::new(callback_ref);

            Some(Box::new(move |progress: ExtractionProgress| -> bool {
                let mut env = match jvm.attach_current_thread() {
                    Ok(env) => env,
                    Err(_) => return false,
                };

                match call_java_progress_callback(&mut env, callback_ref.as_obj(), progress) {
                    Ok(should_continue) => should_continue,
                    Err(_) => false,
                }
            }))
        } else {
            None
        };

        extract_partition_remote_zip(
            url_str,
            &partition,
            output,
            user_agent_str.as_deref(),
            progress_callback,
        )
        .map_err(|e| format!("Failed to extract partition: {}", e))
    }));

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = env.throw_new("java/lang/RuntimeException", e);
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                "Panic occurred in native code",
            );
        }
    }
}

/// extract a partition from a remote payload.bin file (not in ZIP)
///
/// java signature:
/// public static native void extractPartitionRemoteBin(
///     String url,
///     String partitionName,
///     String outputPath,
///     String userAgent,
///     ProgressCallback callback
/// );
#[unsafe(no_mangle)]
pub extern "system" fn Java_com_rhythmcache_payloaddumper_PayloadDumper_extractPartitionRemoteBin(
    mut env: JNIEnv,
    _class: JClass,
    url: JString,
    partition_name: JString,
    output_path: JString,
    user_agent: JString,
    callback: JObject,
) {
    let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let url_str = jstring_to_string(&mut env, &url)?;
        let partition = jstring_to_string(&mut env, &partition_name)?;
        let output = jstring_to_string(&mut env, &output_path)?;
        let user_agent_str = if !user_agent.is_null() {
            Some(jstring_to_string(&mut env, &user_agent)?)
        } else {
            None
        };

        let progress_callback: Option<ProgressCallback> = if !callback.is_null() {
            let callback_ref = env
                .new_global_ref(&callback)
                .map_err(|e| format!("Failed to create global ref: {}", e))?;

            let jvm = env
                .get_java_vm()
                .map_err(|e| format!("Failed to get JavaVM: {}", e))?;

            let callback_ref = Arc::new(callback_ref);

            Some(Box::new(move |progress: ExtractionProgress| -> bool {
                let mut env = match jvm.attach_current_thread() {
                    Ok(env) => env,
                    Err(_) => return false,
                };

                match call_java_progress_callback(&mut env, callback_ref.as_obj(), progress) {
                    Ok(should_continue) => should_continue,
                    Err(_) => false,
                }
            }))
        } else {
            None
        };

        extract_partition_remote_bin(
            url_str,
            &partition,
            output,
            user_agent_str.as_deref(),
            progress_callback,
        )
        .map_err(|e| format!("Failed to extract partition: {}", e))
    }));

    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => {
            let _ = env.throw_new("java/lang/RuntimeException", e);
        }
        Err(_) => {
            let _ = env.throw_new(
                "java/lang/RuntimeException",
                "Panic occurred in native code",
            );
        }
    }
}
