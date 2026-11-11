use std::fs::File;
use std::os::fd::{FromRawFd, RawFd};

use bonk_catalog::manifest::{self, CatalogReport, FamilyEntry, TRUSTED_KEYS};
use bonk_core::msc::{BulkTransport, UsbDisk};
use bonk_core::{BlockDevice, BlockError, partition};
use bonk_hal::usbfs::UsbfsTransport;
use jni::errors::Error;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::{Env, EnvUnowned};

use crate::{bench, screen, vault, writer};

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

/// Size and partitions: what the screen shows before offering to erase the key.
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
) {
    env.with_env(|env| -> Result<_, Error> {
        let lang = text_of(env, &lang)?;
        let theme = text_of(env, &theme)?;
        crate::settings::start(&lang, &theme, system_french, show_all);
        Ok(())
    })
    .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}

/// Checks the signature of the catalog Kotlin downloaded (system HTTPS) and hands it to the screen. Without
/// network, `manifest::load` falls back to the last verified copy.
fn load_catalog(dir: &str, json: Vec<u8>, signature: Vec<u8>) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let fetch = move |address: &str| -> Result<Vec<u8>, String> {
        let bytes = if address.ends_with(".sig") {
            &signature
        } else {
            &json
        };
        if bytes.is_empty() {
            Err("no network".to_owned())
        } else {
            Ok(bytes.clone())
        }
    };
    let base = "https://catalog.bonk-os.app/v1/";
    let loaded = manifest::load(base, std::path::Path::new(dir), now, &TRUSTED_KEYS, &fetch)
        .map_err(|error| error.to_string())?;
    let mut catalog = loaded.catalog;
    vault::set_grub(base, catalog.grub.take());
    let mut images = catalog.images;
    images.sort_by(|a, b| {
        manifest::version_numbers(&b.version).cmp(&manifest::version_numbers(&a.version))
    });
    let report = CatalogReport {
        schema_version: catalog.schema_version,
        sequence: catalog.sequence,
        expires_unix: catalog.expires_unix,
        saved_copy: loaded.saved_copy,
        families: catalog
            .families
            .into_iter()
            .map(|family| FamilyEntry {
                id: family.id,
                name: family.name,
                category: family.category,
                icon_path: None,
            })
            .collect(),
        images,
    };
    let systems = bonk_ui::catalog::systems(&report);
    let mut model = screen::lock(&screen::MODEL);
    let _ = model.update(bonk_ui::model::Message::CatalogLoaded(Ok(systems)));
    Ok(())
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
        let result = load_catalog(&dir, json, signature).map(|()| serde_json::json!({"ok": true}));
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
