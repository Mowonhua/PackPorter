use packporter::infra::zip_archive::{pack_files, unpack_to};
use std::fs;

#[test]
fn compressed_assets_are_stored_and_text_still_compresses_losslessly() {
    let root = std::env::temp_dir().join(format!("packporter-zip-policy-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let names = [
        "region.mca",
        "external.mcc",
        "resources.ZIP",
        "mod.jar",
        "image.png",
        "options.txt",
        "empty.txt",
    ];
    let text = b"renderDistance:12\n".repeat(10_000);
    let files: Vec<_> = names
        .iter()
        .map(|name| {
            let path = root.join(name);
            fs::write(
                &path,
                if *name == "empty.txt" {
                    &[]
                } else {
                    text.as_slice()
                },
            )
            .unwrap();
            path
        })
        .collect();
    let archive = root.join("backup.zip");
    let mut events = Vec::new();
    assert_eq!(
        pack_files(&files, &root, &archive, &mut |done, total| events
            .push((done, total)))
        .unwrap(),
        names.len()
    );
    assert_eq!(events.last(), Some(&(names.len(), names.len())));
    let mut zip = zip::ZipArchive::new(fs::File::open(&archive).unwrap()).unwrap();
    for name in &names[..5] {
        assert_eq!(
            zip.by_name(name).unwrap().compression(),
            zip::CompressionMethod::Stored,
            "已压缩资产不应重复压缩: {name}"
        );
    }
    let entry = zip.by_name("options.txt").unwrap();
    assert!(entry.compressed_size() < entry.size() / 10);
    drop(entry);
    drop(zip);
    let restored = root.join("restored");
    let report = unpack_to(&archive, &restored).unwrap();
    assert_eq!(report.restored, names.len());
    assert_eq!(report.failed, 0);
    for file in files {
        assert_eq!(
            fs::read(restored.join(file.file_name().unwrap())).unwrap(),
            fs::read(file).unwrap()
        );
    }
    fs::remove_dir_all(root).unwrap();
}
