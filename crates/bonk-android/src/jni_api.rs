//! Méthodes natives de `app.bonkos.bonk.Native`. Une erreur revient en JSON, jamais en exception : l'écran
//! l'affiche telle quelle.

use std::fs::File;
use std::os::fd::{FromRawFd, RawFd};

use bonk_core::msc::{MassStorage, UsbDisk};
use bonk_hal::usbfs::UsbfsTransport;
use jni::errors::Error;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jint, jlong};
use jni::{Env, EnvUnowned};

use crate::bench;

/// Copie le descripteur : celui de l'application reste à l'application, et se referme avec sa connexion.
fn transport(fd: RawFd, descriptors: &[u8]) -> Result<UsbfsTransport, String> {
    // SAFETY : `dup` ne touche qu'au descripteur passé, et un échec est contrôlé juste après.
    let copy = unsafe { libc::dup(fd) };
    if copy < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY : `copy` vient d'être créé par `dup` et n'a pas d'autre propriétaire.
    let device = unsafe { File::from_raw_fd(copy) };
    UsbfsTransport::from_claimed(device, descriptors).map_err(|error| error.to_string())
}

fn answer<'caller>(
    env: &mut Env<'caller>,
    result: Result<serde_json::Value, String>,
) -> Result<JString<'caller>, Error> {
    let value = result.unwrap_or_else(|error| serde_json::json!({ "error": error }));
    env.new_string(value.to_string())
}

fn probe(fd: RawFd, descriptors: &[u8]) -> Result<serde_json::Value, String> {
    let mut storage = MassStorage::new(transport(fd, descriptors)?);
    let inquiry = storage.inquiry().map_err(|error| error.to_string())?;
    storage.wait_ready().map_err(|error| error.to_string())?;
    let (blocks, block) = storage.capacity().map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "vendor": inquiry.vendor,
        "product": inquiry.product,
        "blocks": blocks,
        "block_size": block,
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
