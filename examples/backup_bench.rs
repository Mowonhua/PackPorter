//! 固定种子的备份性能回路；数据和产物仅写入本进程的临时目录。
use packporter::infra::zip_archive::{pack_files, unpack_to};
use std::{
    fs,
    io::{BufWriter, Seek, Write},
    path::Path,
    time::Instant,
};

// 保留优化前策略作为对照，并分别隔离输出缓冲、压缩等级的影响。
fn deflate_to(output: impl Write + Seek, source: &Path, level: Option<i64>) {
    let mut writer = zip::ZipWriter::new(output);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .compression_level(level);
    writer
        .start_file(source.file_name().unwrap().to_str().unwrap(), options)
        .unwrap();
    std::io::copy(&mut fs::File::open(source).unwrap(), &mut writer).unwrap();
    writer.finish().unwrap().flush().unwrap();
}

fn main() {
    let root = std::env::temp_dir().join(format!("packporter-bench-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let mut seed = 42u64;
    let mut bytes = vec![0u8; 16 * 1024 * 1024];
    for byte in &mut bytes {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        *byte = seed as u8;
    }
    // 随机字节模拟已压缩的区域块/资源包；文本用于检查压缩收益是否保留。
    for (name, data) in [
        ("region.mca", bytes.as_slice()),
        (
            "config.toml",
            b"setting = true\n".repeat(100_000).as_slice(),
        ),
    ] {
        let source = root.join(name);
        fs::write(&source, data).unwrap();
        let archive = root.join(format!("{name}.zip"));
        for variant in ["original", "buffered", "fast"] {
            let output = fs::File::create(&archive).unwrap();
            let start = Instant::now();
            if variant == "original" {
                deflate_to(output, &source, None);
            } else {
                deflate_to(
                    BufWriter::with_capacity(256 * 1024, output),
                    &source,
                    if variant == "fast" { Some(1) } else { None },
                );
            }
            println!(
                "{name} {variant}: {:.3}s, {} bytes",
                start.elapsed().as_secs_f64(),
                fs::metadata(&archive).unwrap().len()
            );
        }
        let start = Instant::now();
        pack_files(&[source.clone()], &root, &archive, &mut |_, _| {}).unwrap();
        println!(
            "{name} optimized: {:.3}s, {} -> {} bytes",
            start.elapsed().as_secs_f64(),
            data.len(),
            fs::metadata(&archive).unwrap().len()
        );
        let restored = root.join("restored");
        assert_eq!(unpack_to(&archive, &restored).unwrap().restored, 1);
        assert_eq!(fs::read(restored.join(name)).unwrap(), data);
    }
    fs::remove_dir_all(root).unwrap();
}
