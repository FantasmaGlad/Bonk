//! Banc d'essai du transport `usbfs` sur une vraie clé, hors du binaire publié.
//! `essai_usbfs <périphérique> info`
//! `essai_usbfs <périphérique> read <décalage> <longueur> <fichier>`
//! `essai_usbfs <périphérique> write <décalage> <fichier>`
//! Aucun filtre de sécurité ici : c'est à l'appelant de désigner la clé de test.

#[cfg(any(target_os = "linux", target_os = "android"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use bonk_core::block::BlockDevice;
    use bonk_core::msc::{MassStorage, UsbDisk};
    use bonk_hal::usbfs::UsbfsTransport;
    use std::path::Path;
    use std::time::Instant;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let [path, command, rest @ ..] = args.as_slice() else {
        return Err("usage: essai_usbfs <device> info|read|write ...".into());
    };
    let path = Path::new(path);
    if *command == "info" {
        let mut storage = MassStorage::new(UsbfsTransport::open(path)?);
        println!("{:?}", storage.inquiry()?);
        storage.wait_ready()?;
        println!("capacity {:?}", storage.capacity()?);
        return Ok(());
    }
    let mut disk = UsbDisk::open(UsbfsTransport::open(path)?)?;
    let started = Instant::now();
    let bytes = match *command {
        "read" => {
            let [offset, length, out] = rest else {
                return Err("read <offset> <length> <file>".into());
            };
            let mut data = vec![0; length.parse()?];
            disk.read_at(offset.parse()?, &mut data)?;
            std::fs::write(out, &data)?;
            data.len()
        }
        "write" => {
            let [offset, input] = rest else {
                return Err("write <offset> <file>".into());
            };
            let data = std::fs::read(input)?;
            disk.write_at(offset.parse()?, &data)?;
            disk.flush()?;
            data.len()
        }
        _ => return Err("unknown command".into()),
    };
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "{command} {bytes} bytes in {seconds:.2} s, {:.1} MB/s",
        bytes as f64 / seconds / 1e6
    );
    Ok(())
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn main() {
    eprintln!("usbfs exists on Linux and Android only");
}
