use std::fs::File;
use std::os::fd::{FromRawFd, RawFd};

use bonk_core::msc::{BulkTransport, UsbDisk};
use bonk_core::{BlockDevice, BlockError, partition};
use bonk_hal::usbfs::UsbfsTransport;
use jni::errors::Error;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::{Env, EnvUnowned};

use crate::{bench, catalog, screen, vault, writer};

/// Duplicates the descriptor: the app's own stays with the app and closes with its connection.
fn transport(fd: RawFd, descriptors: &[u8]) -> Result<UsbfsTransport, String> {
    // SAFETY: `dup` only touches the given descriptor, and a failure is checked right after.
    let copy = unsafe { libc::dup(fd) };
    if copy < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: `copy` was just created by `dup` and has no other owner.
    let device = unsafe { File::from_raw_fd(copy) };
    let mut transport =
        UsbfsTransport::from_claimed(device, descriptors).map_err(|error| error.to_string())?;
    // An interrupted exchange (app killed, key given back) leaves the key waiting for a sequel nobody sends.
    // Some keys refuse this reset without needing it: a failure stops nothing.
    let _ = transport.reset();
    Ok(transport)
}

fn answer<'caller>(
    env: &mut Env<'caller>,
    result: Result<serde_json::Value, String>,
) -> Result<JString<'caller>, Error> {
    let value = result.unwrap_or_else(|error| serde_json::json!({ "error": error }));
    env.new_string(value.to_string())
}

fn probe(fd: RawFd, descriptors: &[u8]) -> Result<serde_json::Value, String> {
    let disk = UsbDisk::open(transport(fd, descriptors)?).map_err(|error| error.to_string())?;
    let layout = partition::inspect(&disk).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "size_bytes": disk.size(),
        "partitions": screen::partitions_of(&layout),
    }))
}

fn open(fd: RawFd, descriptors: &[u8]) -> Result<UsbDisk<UsbfsTransport>, String> {
    UsbDisk::open(transport(fd, descriptors)?).map_err(|error| error.to_string())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_probe<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    fd: jint,
    descriptors: JByteArray<'caller>,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> {
        let descriptors = env.convert_byte_array(&descriptors)?;
        let result = probe(fd, &descriptors);
        answer(env, result)
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_readHash<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    fd: jint,
    descriptors: JByteArray<'caller>,
    offset: jlong,
    length: jint,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> {
        let descriptors = env.convert_byte_array(&descriptors)?;
        let result = open(fd, &descriptors).and_then(|disk| {
            let offset = u64::try_from(offset).map_err(|error| error.to_string())?;
            let length = usize::try_from(length).map_err(|error| error.to_string())?;
            bench::read_hash(&disk, offset, length).map_err(|error| error.to_string())
        });
        answer(env, result)
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[cfg(feature = "banc")]
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_writeBench<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    fd: jint,
    descriptors: JByteArray<'caller>,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> {
        let descriptors = env.convert_byte_array(&descriptors)?;
        let result = open(fd, &descriptors)
            .and_then(|mut disk| bench::write_bench(&mut disk).map_err(|error| error.to_string()));
        answer(env, result)
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

fn text_of<'caller>(env: &mut Env<'caller>, value: &JString<'caller>) -> Result<String, Error> {
    value.try_to_string(env)
}

/// A message for the shared screen: the state to draw and the effects to perform.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_dispatch<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    message: JString<'caller>,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> {
        let message = text_of(env, &message)?;
        env.new_string(screen::dispatch(&message))
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// The choices kept in the app's preferences (`auto`, `en`, `fr`; `auto`, `light`, `dark`), and the
/// system's language for `auto`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_start<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    lang: JString<'caller>,
    theme: JString<'caller>,
    system_french: jboolean,
    show_all: jboolean,
    offline: jboolean,
    incognito: jboolean,
    notify: jboolean,
) {
    env.with_env(|env| -> Result<_, Error> {
        let lang = text_of(env, &lang)?;
        let theme = text_of(env, &theme)?;
        crate::settings::start(
            &lang,
            &theme,
            system_french,
            show_all,
            offline,
            incognito,
            notify,
        );
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_notice<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> { env.new_string(crate::settings::notice()) })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_catalogAddresses<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> { env.new_string(catalog::addresses().to_string()) })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_loadCatalog<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    dir: JString<'caller>,
    json: JByteArray<'caller>,
    signature: JByteArray<'caller>,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> {
        let dir = text_of(env, &dir)?;
        let json = env.convert_byte_array(&json)?;
        let signature = env.convert_byte_array(&signature)?;
        let result = catalog::load(&dir, json, signature).map(|()| serde_json::json!({"ok": true}));
        answer(env, result)
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// The logos still to download, as `[{"url", "sha256"}]`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_missingIcons<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    dir: JString<'caller>,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> {
        let dir = text_of(env, &dir)?;
        env.new_string(catalog::missing_icons(&dir).to_string())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// Kept and shown only if the bytes match the signed hash.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_storeIcon<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    dir: JString<'caller>,
    sha256: JString<'caller>,
    bytes: JByteArray<'caller>,
) -> JString<'caller> {
    env.with_env(|env| -> Result<_, Error> {
        let dir = text_of(env, &dir)?;
        let sha256 = text_of(env, &sha256)?;
        let bytes = env.convert_byte_array(&bytes)?;
        let result =
            catalog::store_icon(&dir, &sha256, &bytes).map(|()| serde_json::json!({"ok": true}));
        answer(env, result)
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// Blocks until the write ends: call off the UI thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_write<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    fd: jint,
    descriptors: JByteArray<'caller>,
    disk_id: JString<'caller>,
    device: JString<'caller>,
    serial: JString<'caller>,
    source: JString<'caller>,
    sha256: JString<'caller>,
    raw: jboolean,
    mbr: jboolean,
    hybrid: jboolean,
) {
    env.with_env(|env| -> Result<_, Error> {
        let descriptors = env.convert_byte_array(&descriptors)?;
        let disk_id = text_of(env, &disk_id)?;
        let device = text_of(env, &device)?;
        let serial = text_of(env, &serial)?;
        let source = text_of(env, &source)?;
        let sha256 = text_of(env, &sha256)?;
        let request = writer::Request {
            disk_id: &disk_id,
            device: &device,
            serial: &serial,
            source: &source,
            sha256: (!sha256.is_empty()).then_some(sha256.clone()),
            raw,
            mbr,
            hybrid,
        };
        writer::write(
            &request,
            move || -> Result<Box<dyn BlockDevice>, BlockError> {
                let transport = transport(fd, &descriptors)
                    .map_err(|error| BlockError::Io(std::io::Error::other(error)))?;
                Ok(Box::new(UsbDisk::open(transport)?))
            },
        );
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// Blocks until the vault action ends: call off the UI thread.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_vault<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    fd: jint,
    descriptors: JByteArray<'caller>,
    action: JString<'caller>,
    disk_id: JString<'caller>,
    device: JString<'caller>,
    serial: JString<'caller>,
    iso: JString<'caller>,
    name: JString<'caller>,
    grub: JString<'caller>,
) {
    env.with_env(|env| -> Result<_, Error> {
        let descriptors = env.convert_byte_array(&descriptors)?;
        let action = text_of(env, &action)?;
        let disk_id = text_of(env, &disk_id)?;
        let device = text_of(env, &device)?;
        let serial = text_of(env, &serial)?;
        let iso = text_of(env, &iso)?;
        let name = text_of(env, &name)?;
        let grub = text_of(env, &grub)?;
        let request = vault::Request {
            action: &action,
            disk_id: &disk_id,
            device: &device,
            serial: &serial,
            iso: &iso,
            name: &name,
            grub: &grub,
        };
        vault::run(
            &request,
            move || -> Result<Box<dyn BlockDevice>, BlockError> {
                let transport = transport(fd, &descriptors)
                    .map_err(|error| BlockError::Io(std::io::Error::other(error)))?;
                Ok(Box::new(UsbDisk::open(transport)?))
            },
        );
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
