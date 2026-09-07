//! Writes a sample FAT32 disk image, for exercising the desktop shell.

use integration_tests::fixture::Fat32Image;
use storage_io::BlockDevice;

fn jpeg(body: usize) -> Vec<u8> {
    let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    v.extend_from_slice(b"JFIF\0");
    v.resize(18, 0);
    v.extend(std::iter::repeat_n(0x41u8, body));
    v.extend_from_slice(&[0xFF, 0xD9]);
    v
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "sample.img".into());
    let mut image = Fat32Image::build();

    // An active photo.
    let photo = jpeg(400);
    image.set_fat(3, 0x0FFF_FFFF);
    image.put_cluster_data(3, &photo);
    image.put_root_entry(
        0,
        &Fat32Image::dir_entry(b"PHOTO   JPG", 3, photo.len() as u32, false),
    );

    // A deleted document, its chain released.
    let notes = b"Quarterly figures. Deleted before the drive was imaged.";
    image.put_cluster_data(5, notes);
    image.put_root_entry(
        1,
        &Fat32Image::dir_entry(b"NOTES   TXT", 5, notes.len() as u32, true),
    );

    // An active file whose content does not match its extension.
    let mislabelled = b"plain text pretending to be an image";
    image.set_fat(7, 0x0FFF_FFFF);
    image.put_cluster_data(7, mislabelled);
    image.put_root_entry(
        2,
        &Fat32Image::dir_entry(b"FAKE    JPG", 7, mislabelled.len() as u32, false),
    );

    let device = image.into_device();
    let bytes = device.snapshot();
    let capacity = device.capacity();
    std::fs::write(&path, bytes).expect("write image");
    println!("wrote {path} ({capacity} bytes)");
}
