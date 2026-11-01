//! Méthodes natives de `app.bonkos.bonk.Native`. Une erreur revient en JSON, jamais en exception : l'écran
//! l'affiche telle quelle.

use std::fs::File;
use std::os::fd::{FromRawFd, RawFd};

use bonk_catalog::manifest::{self, CatalogReport, FamilyEntry, TRUSTED_KEYS};
use bonk_core::msc::{BulkTransport, MassStorage, UsbDisk};
use bonk_core::{BlockDevice, BlockError};
use bonk_hal::usbfs::UsbfsTransport;
use jni::errors::Error;
use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::{Env, EnvUnowned};

use crate::{bench, screen, writer};

/// Copie le descripteur : celui de l'application reste à l'application, et se referme avec sa connexion.
fn transport(fd: RawFd, descriptors: &[u8]) -> Result<UsbfsTransport, String> {
    // SAFETY : `dup` ne touche qu'au descripteur passé, et un échec est contrôlé juste après.
    let copy = unsafe { libc::dup(fd) };
    if copy < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY : `copy` vient d'être créé par `dup` et n'a pas d'autre propriétaire.
    let device = unsafe { File::from_raw_fd(copy) };
    let mut transport =
        UsbfsTransport::from_claimed(device, descriptors).map_err(|error| error.to_string())?;
    // Un échange interrompu (application tuée, clé rendue au système) laisse la clé en attente d'une suite que
    // personne n'enverra. Certaines clés refusent cette remise à zéro sans en avoir besoin : l'échec n'arrête rien.
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

fn text_of<'caller>(env: &mut Env<'caller>, value: &JString<'caller>) -> Result<String, Error> {
    value.try_to_string(env)
}

/// Un message pour l'écran commun : l'état à dessiner et les effets à accomplir.
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

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_setLanguage<'caller>(
    _env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    french: jboolean,
) {
    screen::set_language(french);
}

/// Vérifie la signature du catalogue que Kotlin a téléchargé (HTTPS du système) et le donne à l'écran. Sans
/// réseau, `manifest::load` reprend la dernière copie vérifiée.
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
    let catalog = loaded.catalog;
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
    let mut model = screen::MODEL.lock().map_err(|error| error.to_string())?;
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

/// Bloque jusqu'à la fin de l'écriture : à appeler hors du fil de l'écran.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_bonkos_bonk_Native_write<'caller>(
    mut env: EnvUnowned<'caller>,
    _class: JClass<'caller>,
    fd: jint,
    descriptors: JByteArray<'caller>,
    disk_id: JString<'caller>,
    source: JString<'caller>,
    sha256: JString<'caller>,
    raw: jboolean,
    mbr: jboolean,
) {
    env.with_env(|env| -> Result<_, Error> {
        let descriptors = env.convert_byte_array(&descriptors)?;
        let disk_id = text_of(env, &disk_id)?;
        let source = text_of(env, &source)?;
        let sha256 = text_of(env, &sha256)?;
        let request = writer::Request {
            disk_id: &disk_id,
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
