use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Cursor,
    path::{Component, Path, PathBuf},
};

const UNITY_FS_SIGNATURE: &[u8] = b"UnityFS\0";
const COMPRESSION_MASK: u32 = 0x3f;
const BLOCKS_INFO_AT_END: u32 = 0x80;
const BLOCK_INFO_NEEDS_PADDING: u32 = 0x200;
const MAX_BLOCK_COUNT: usize = 1_000_000;
const MAX_NODE_COUNT: usize = 1_000_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnityExtractionReport {
    pub files_scanned: usize,
    pub bundles_extracted: usize,
    pub bundle_entries: usize,
    pub passthrough_files: usize,
    pub extracted_bytes: u64,
}

#[derive(Debug)]
struct UnityFsEntry {
    path: String,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct BlockInfo {
    uncompressed_size: usize,
    compressed_size: usize,
    flags: u16,
}

#[derive(Debug)]
struct NodeInfo {
    offset: usize,
    size: usize,
    path: String,
}

pub fn extract_unity_content(
    unity_data_dir: &Path,
    output: &Path,
) -> Result<UnityExtractionReport> {
    if !unity_data_dir.is_dir() {
        bail!(
            "Unity data directory does not exist: {}",
            unity_data_dir.display()
        );
    }

    if output.starts_with(unity_data_dir) {
        bail!(
            "asset extraction output must be outside the Unity data directory: {}",
            output.display()
        );
    }

    if output.exists() && fs::read_dir(output)?.next().transpose()?.is_some() {
        bail!("asset extraction output is not empty: {}", output.display());
    }
    fs::create_dir_all(output)?;

    let files = collect_files(unity_data_dir)?;
    let mut report = UnityExtractionReport {
        files_scanned: files.len(),
        bundles_extracted: 0,
        bundle_entries: 0,
        passthrough_files: 0,
        extracted_bytes: 0,
    };

    for source in files {
        let relative = source
            .strip_prefix(unity_data_dir)
            .with_context(|| format!("resolving Unity input path {}", source.display()))?;
        let bytes = fs::read(&source)
            .with_context(|| format!("reading Unity data file {}", source.display()))?;

        if is_unityfs(&bytes) {
            let entries = parse_unityfs(&bytes)
                .with_context(|| format!("parsing UnityFS bundle {}", source.display()))?;
            let bundle_root = output.join("bundles").join(relative);

            for entry in entries {
                let entry_path = safe_bundle_path(&entry.path)?;
                let target = bundle_root.join(entry_path);
                write_new_file(&target, &entry.bytes)?;
                report.bundle_entries += 1;
                report.extracted_bytes = report
                    .extracted_bytes
                    .checked_add(entry.bytes.len() as u64)
                    .context("Unity extraction byte count overflow")?;
            }

            report.bundles_extracted += 1;
        } else {
            let target = output.join("raw").join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&source, &target).with_context(|| {
                format!(
                    "copying standalone Unity file {} to {}",
                    source.display(),
                    target.display()
                )
            })?;
            report.passthrough_files += 1;
            report.extracted_bytes = report
                .extracted_bytes
                .checked_add(bytes.len() as u64)
                .context("Unity extraction byte count overflow")?;
        }
    }

    Ok(report)
}

fn is_unityfs(bytes: &[u8]) -> bool {
    bytes.starts_with(UNITY_FS_SIGNATURE)
}

fn parse_unityfs(bytes: &[u8]) -> Result<Vec<UnityFsEntry>> {
    let mut reader = BeReader::new(bytes);
    let signature = reader.read_cstring()?;
    ensure!(
        signature == "UnityFS",
        "unsupported Unity bundle signature {signature:?}"
    );

    let format = reader.read_u32()?;
    ensure!(format >= 6, "unsupported UnityFS format version {format}");

    let _minimum_player_version = reader.read_cstring()?;
    let _engine_version = reader.read_cstring()?;
    let declared_size =
        usize::try_from(reader.read_u64()?).context("UnityFS size does not fit usize")?;
    ensure!(
        declared_size == 0 || declared_size <= bytes.len(),
        "UnityFS bundle is truncated: header declares {declared_size} bytes, file has {}",
        bytes.len()
    );

    let compressed_info_size = usize::try_from(reader.read_u32()?)
        .context("UnityFS block-info size does not fit usize")?;
    let uncompressed_info_size = usize::try_from(reader.read_u32()?)
        .context("UnityFS block-info size does not fit usize")?;
    let archive_flags = reader.read_u32()?;

    if format >= 7 {
        reader.align(16)?;
    }

    let blocks_start = reader.position();
    let info_at_end = archive_flags & BLOCKS_INFO_AT_END != 0;
    let info_start = if info_at_end {
        bytes
            .len()
            .checked_sub(compressed_info_size)
            .context("UnityFS block-info size exceeds file size")?
    } else {
        blocks_start
    };
    let info_end = info_start
        .checked_add(compressed_info_size)
        .context("UnityFS block-info range overflow")?;
    let compressed_info = bytes
        .get(info_start..info_end)
        .context("UnityFS block-info range exceeds file size")?;
    let info = decompress_data(compressed_info, uncompressed_info_size, archive_flags)
        .context("decompressing UnityFS block info")?;

    let (blocks, nodes) = parse_block_info(&info)?;
    let mut data_position = if info_at_end { blocks_start } else { info_end };
    if archive_flags & BLOCK_INFO_NEEDS_PADDING != 0 {
        data_position = align_offset(data_position, 16)?;
    }

    let total_uncompressed = blocks.iter().try_fold(0_usize, |total, block| {
        total
            .checked_add(block.uncompressed_size)
            .context("UnityFS decompressed size overflow")
    })?;
    let mut storage = Vec::with_capacity(total_uncompressed);

    for block in blocks {
        let block_end = data_position
            .checked_add(block.compressed_size)
            .context("UnityFS data block range overflow")?;
        let compressed = bytes
            .get(data_position..block_end)
            .context("UnityFS data block exceeds file size")?;
        let decompressed =
            decompress_data(compressed, block.uncompressed_size, u32::from(block.flags))
                .context("decompressing UnityFS data block")?;
        storage.extend_from_slice(&decompressed);
        data_position = block_end;
    }

    let mut entries = Vec::with_capacity(nodes.len());
    for node in nodes {
        let end = node
            .offset
            .checked_add(node.size)
            .context("UnityFS node range overflow")?;
        let node_bytes = storage
            .get(node.offset..end)
            .with_context(|| format!("UnityFS node {} exceeds decompressed data", node.path))?;
        entries.push(UnityFsEntry {
            path: node.path,
            bytes: node_bytes.to_vec(),
        });
    }

    Ok(entries)
}

fn parse_block_info(bytes: &[u8]) -> Result<(Vec<BlockInfo>, Vec<NodeInfo>)> {
    let mut reader = BeReader::new(bytes);
    reader.read_exact(16).context("reading UnityFS data hash")?;

    let block_count =
        usize::try_from(reader.read_u32()?).context("UnityFS block count does not fit usize")?;
    ensure!(
        block_count <= MAX_BLOCK_COUNT,
        "UnityFS block count {block_count} exceeds safety limit"
    );

    let mut blocks = Vec::with_capacity(block_count);
    for _ in 0..block_count {
        blocks.push(BlockInfo {
            uncompressed_size: usize::try_from(reader.read_u32()?)
                .context("UnityFS uncompressed block size does not fit usize")?,
            compressed_size: usize::try_from(reader.read_u32()?)
                .context("UnityFS compressed block size does not fit usize")?,
            flags: reader.read_u16()?,
        });
    }

    let node_count =
        usize::try_from(reader.read_u32()?).context("UnityFS node count does not fit usize")?;
    ensure!(
        node_count <= MAX_NODE_COUNT,
        "UnityFS node count {node_count} exceeds safety limit"
    );

    let mut nodes = Vec::with_capacity(node_count);
    for _ in 0..node_count {
        let offset = usize::try_from(reader.read_u64()?)
            .context("UnityFS node offset does not fit usize")?;
        let size =
            usize::try_from(reader.read_u64()?).context("UnityFS node size does not fit usize")?;
        let _flags = reader.read_u32()?;
        let path = reader.read_cstring()?;
        nodes.push(NodeInfo { offset, size, path });
    }

    Ok((blocks, nodes))
}

fn decompress_data(input: &[u8], expected_size: usize, flags: u32) -> Result<Vec<u8>> {
    let compression = flags & COMPRESSION_MASK;
    let output = match compression {
        0 => input.to_vec(),
        1 => {
            let mut output = Vec::with_capacity(expected_size);
            lzma_rs::lzma_decompress(&mut Cursor::new(input), &mut output)
                .context("decompressing LZMA data")?;
            output
        }
        2 | 3 => {
            lz4_flex::block::decompress(input, expected_size).context("decompressing LZ4 data")?
        }
        4 => bail!("UnityFS LZHAM compression is not supported"),
        other => bail!("unknown UnityFS compression type {other}"),
    };

    ensure!(
        output.len() == expected_size,
        "UnityFS decompression size mismatch: expected {expected_size}, got {}",
        output.len()
    );
    Ok(output)
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();

    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .with_context(|| format!("reading Unity directory {}", directory.display()))?
        {
            let entry = entry?;
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() && should_extract_file(root, &entry.path()) {
                files.push(entry.path());
            }
        }
    }

    files.sort();
    Ok(files)
}

fn should_extract_file(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };

    relative
        .components()
        .next()
        .is_none_or(|component| component.as_os_str() != "Managed")
}

fn safe_bundle_path(path: &str) -> Result<PathBuf> {
    let normalized = path.replace('\\', "/");
    let path = Path::new(&normalized);
    ensure!(!path.as_os_str().is_empty(), "UnityFS node path is empty");
    ensure!(
        !path.is_absolute(),
        "unsafe UnityFS node path: {normalized}"
    );

    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => safe.push(value),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                bail!("unsafe UnityFS node path: {normalized}")
            }
        }
    }

    ensure!(!safe.as_os_str().is_empty(), "UnityFS node path is empty");
    Ok(safe)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.exists() {
        bail!("Unity extraction path collision: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, bytes)
        .with_context(|| format!("writing extracted Unity file {}", path.display()))
}

fn align_offset(value: usize, alignment: usize) -> Result<usize> {
    ensure!(
        alignment.is_power_of_two(),
        "alignment must be a power of two"
    );
    let mask = alignment - 1;
    value
        .checked_add(mask)
        .map(|aligned| aligned & !mask)
        .context("UnityFS alignment overflow")
}

struct BeReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> BeReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn position(&self) -> usize {
        self.position
    }

    fn align(&mut self, alignment: usize) -> Result<()> {
        let aligned = align_offset(self.position, alignment)?;
        ensure!(
            aligned <= self.bytes.len(),
            "UnityFS alignment exceeds file size"
        );
        self.position = aligned;
        Ok(())
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .context("UnityFS reader offset overflow")?;
        let value = self
            .bytes
            .get(self.position..end)
            .context("unexpected end of UnityFS data")?;
        self.position = end;
        Ok(value)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes: [u8; 2] = self.read_exact(2)?.try_into().expect("fixed-size read");
        Ok(u16::from_be_bytes(bytes))
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self.read_exact(4)?.try_into().expect("fixed-size read");
        Ok(u32::from_be_bytes(bytes))
    }

    fn read_u64(&mut self) -> Result<u64> {
        let bytes: [u8; 8] = self.read_exact(8)?.try_into().expect("fixed-size read");
        Ok(u64::from_be_bytes(bytes))
    }

    fn read_cstring(&mut self) -> Result<String> {
        let tail = self
            .bytes
            .get(self.position..)
            .context("UnityFS string starts past end of file")?;
        let length = tail
            .iter()
            .position(|byte| *byte == 0)
            .context("unterminated UnityFS string")?;
        let raw = self.read_exact(length)?;
        self.read_exact(1)?;
        Ok(std::str::from_utf8(raw)
            .context("UnityFS string is not UTF-8")?
            .to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_uncompressed_unityfs_bundle() {
        let payload = b"hello unity";
        let bundle = build_test_bundle(payload, false);
        let entries = parse_unityfs(&bundle).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, "CAB-test");
        assert_eq!(entries[0].bytes, payload);
    }

    #[test]
    fn parses_lz4_unityfs_block() {
        let payload =
            b"a UnityFS payload that compresses well a UnityFS payload that compresses well";
        let bundle = build_test_bundle(payload, true);
        let entries = parse_unityfs(&bundle).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].bytes, payload);
    }

    #[test]
    fn extracts_bundles_and_preserves_standalone_files() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("Data");
        let output = temp.path().join("extracted");
        fs::create_dir_all(&input).unwrap();
        fs::write(input.join("bundle"), build_test_bundle(b"atlas", false)).unwrap();
        fs::write(input.join("sharedassets0.assets"), b"serialized").unwrap();

        let report = extract_unity_content(&input, &output).unwrap();

        assert_eq!(report.files_scanned, 2);
        assert_eq!(report.bundles_extracted, 1);
        assert_eq!(report.bundle_entries, 1);
        assert_eq!(report.passthrough_files, 1);
        assert_eq!(
            fs::read(output.join("bundles/bundle/CAB-test")).unwrap(),
            b"atlas"
        );
        assert_eq!(
            fs::read(output.join("raw/sharedassets0.assets")).unwrap(),
            b"serialized"
        );
    }

    #[test]
    fn rejects_parent_directory_node_paths() {
        assert!(safe_bundle_path("../escape").is_err());
        assert!(safe_bundle_path("..\\escape").is_err());
    }

    fn build_test_bundle(payload: &[u8], lz4: bool) -> Vec<u8> {
        let compressed_payload = if lz4 {
            lz4_flex::block::compress(payload)
        } else {
            payload.to_vec()
        };
        let block_flags = if lz4 { 2_u16 } else { 0_u16 };

        let mut info = vec![0_u8; 16];
        push_u32(&mut info, 1);
        push_u32(&mut info, payload.len() as u32);
        push_u32(&mut info, compressed_payload.len() as u32);
        info.extend_from_slice(&block_flags.to_be_bytes());
        push_u32(&mut info, 1);
        info.extend_from_slice(&0_u64.to_be_bytes());
        info.extend_from_slice(&(payload.len() as u64).to_be_bytes());
        push_u32(&mut info, 0);
        push_cstring(&mut info, "CAB-test");

        let mut bundle = Vec::new();
        push_cstring(&mut bundle, "UnityFS");
        push_u32(&mut bundle, 6);
        push_cstring(&mut bundle, "5.x.x");
        push_cstring(&mut bundle, "6000.3.12f1");
        let size_offset = bundle.len();
        bundle.extend_from_slice(&0_u64.to_be_bytes());
        push_u32(&mut bundle, info.len() as u32);
        push_u32(&mut bundle, info.len() as u32);
        push_u32(&mut bundle, 0x40);
        bundle.extend_from_slice(&info);
        bundle.extend_from_slice(&compressed_payload);

        let total_size = bundle.len() as u64;
        bundle[size_offset..size_offset + 8].copy_from_slice(&total_size.to_be_bytes());
        bundle
    }

    fn push_u32(bytes: &mut Vec<u8>, value: u32) {
        bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn push_cstring(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }
}
