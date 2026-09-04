use anyhow::{Context, Result, bail, ensure};
use std::collections::BTreeSet;

pub(crate) const CLASS_ID_TEXTURE2D: i32 = 28;
pub(crate) const CLASS_ID_TEXT_ASSET: i32 = 49;

const MIN_MODERN_SERIALIZED_VERSION: u32 = 22;
const MAX_SERIALIZED_VERSION: u32 = 30;
const MAX_TYPE_COUNT: usize = 1_000_000;
const MAX_OBJECT_COUNT: usize = 10_000_000;
const MAX_TYPE_TREE_NODES: usize = 1_000_000;
const MAX_TYPE_TREE_STRINGS: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Endian {
    Little,
    Big,
}

#[derive(Debug, Clone)]
pub(crate) struct UnityVersion {
    pub major: u32,
    pub minor: u32,
}

impl UnityVersion {
    pub(crate) fn parse(raw: String) -> Self {
        let mut numbers = raw
            .split(|character: char| !character.is_ascii_digit())
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.parse::<u32>().ok());

        Self {
            major: numbers.next().unwrap_or(0),
            minor: numbers.next().unwrap_or(0),
        }
    }

    pub(crate) fn at_least(&self, major: u32, minor: u32) -> bool {
        self.major > major || (self.major == major && self.minor >= minor)
    }

    pub(crate) fn before(&self, major: u32, minor: u32) -> bool {
        self.major < major || (self.major == major && self.minor < minor)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SerializedType {
    pub class_id: i32,
    pub fields: BTreeSet<String>,
}

impl SerializedType {
    pub(crate) fn has_field(&self, field: &str) -> bool {
        self.fields.contains(field)
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ObjectInfo {
    pub path_id: i64,
    pub byte_start: usize,
    pub byte_size: usize,
    pub type_index: usize,
    pub class_id: i32,
}

#[derive(Debug)]
pub(crate) struct SerializedFile<'a> {
    pub endian: Endian,
    pub unity_version: UnityVersion,
    pub target_platform: i32,
    pub enable_type_tree: bool,
    pub types: Vec<SerializedType>,
    pub objects: Vec<ObjectInfo>,
    pub(crate) bytes: &'a [u8],
}

impl<'a> SerializedFile<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self> {
        let header = parse_header(bytes)?;
        let mut reader = Reader::with_position(bytes, header.endian, header.header_size)?;

        let unity_version = UnityVersion::parse(reader.read_cstring()?);
        let target_platform = reader.read_i32()?;
        let enable_type_tree = reader.read_bool()?;
        let type_count = reader.read_count("serialized type", MAX_TYPE_COUNT)?;
        let mut types = Vec::with_capacity(type_count);

        for _ in 0..type_count {
            types.push(read_serialized_type(
                &mut reader,
                header.version,
                enable_type_tree,
            )?);
        }

        let object_count = reader.read_count("serialized object", MAX_OBJECT_COUNT)?;
        let mut objects = Vec::with_capacity(object_count);
        for _ in 0..object_count {
            reader.align(4)?;
            let path_id = reader.read_i64()?;
            let relative_start = reader.read_i64()?;
            ensure!(
                relative_start >= 0,
                "serialized object {path_id} has a negative byte offset"
            );
            let byte_size = usize::try_from(reader.read_u32()?)
                .context("serialized object byte size does not fit usize")?;
            let type_id = reader.read_i32()?;
            ensure!(
                type_id >= 0,
                "serialized object {path_id} has a negative type index"
            );
            let type_index = usize::try_from(type_id).expect("non-negative i32 fits usize");
            let serialized_type = types.get(type_index).with_context(|| {
                format!("serialized object {path_id} references missing type {type_id}")
            })?;
            let relative_start = usize::try_from(relative_start)
                .context("serialized object byte offset does not fit usize")?;
            let byte_start = header
                .data_offset
                .checked_add(relative_start)
                .context("serialized object byte offset overflow")?;
            let byte_end = byte_start
                .checked_add(byte_size)
                .context("serialized object byte range overflow")?;
            ensure!(
                byte_end <= bytes.len(),
                "serialized object {path_id} exceeds file bounds"
            );

            objects.push(ObjectInfo {
                path_id,
                byte_start,
                byte_size,
                type_index,
                class_id: serialized_type.class_id,
            });
        }

        Ok(Self {
            endian: header.endian,
            unity_version,
            target_platform,
            enable_type_tree,
            types,
            objects,
            bytes,
        })
    }

    pub(crate) fn object_bytes(&self, object: &ObjectInfo) -> Result<&'a [u8]> {
        let end = object
            .byte_start
            .checked_add(object.byte_size)
            .context("serialized object byte range overflow")?;
        self.bytes
            .get(object.byte_start..end)
            .with_context(|| format!("serialized object {} exceeds file bounds", object.path_id))
    }

    pub(crate) fn object_type(&self, object: &ObjectInfo) -> Result<&SerializedType> {
        self.types.get(object.type_index).with_context(|| {
            format!(
                "serialized object {} references missing type {}",
                object.path_id, object.type_index
            )
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct SerializedHeader {
    version: u32,
    data_offset: usize,
    endian: Endian,
    header_size: usize,
}

pub(crate) fn looks_like_serialized_file(bytes: &[u8]) -> bool {
    parse_header(bytes).is_ok()
}

fn parse_header(bytes: &[u8]) -> Result<SerializedHeader> {
    ensure!(
        bytes.len() >= 48,
        "serialized file is smaller than a modern header"
    );
    let mut reader = Reader::new(bytes, Endian::Big);

    let _legacy_metadata_size = reader.read_u32()?;
    let _legacy_file_size = reader.read_u32()?;
    let version = reader.read_u32()?;
    let _legacy_data_offset = reader.read_u32()?;
    ensure!(
        (MIN_MODERN_SERIALIZED_VERSION..=MAX_SERIALIZED_VERSION).contains(&version),
        "unsupported serialized file version {version}"
    );

    let endian = match reader.read_u8()? {
        0 => Endian::Little,
        1 => Endian::Big,
        value => bail!("invalid serialized file endian marker {value}"),
    };
    reader.skip(3)?;

    let metadata_size = usize::try_from(reader.read_u32()?)
        .context("serialized metadata size does not fit usize")?;
    let file_size = reader.read_i64()?;
    let data_offset = reader.read_i64()?;
    let _unknown = reader.read_i64()?;

    ensure!(
        file_size >= 0,
        "serialized file declares a negative file size"
    );
    ensure!(
        data_offset >= 0,
        "serialized file declares a negative data offset"
    );
    let file_size =
        usize::try_from(file_size).context("serialized file size does not fit usize")?;
    let data_offset =
        usize::try_from(data_offset).context("serialized data offset does not fit usize")?;
    ensure!(
        file_size == 0 || file_size <= bytes.len(),
        "serialized file is truncated: declares {file_size} bytes, has {}",
        bytes.len()
    );
    ensure!(
        data_offset >= reader.position(),
        "serialized data starts inside its header"
    );
    ensure!(
        data_offset <= bytes.len(),
        "serialized data offset exceeds file size"
    );
    ensure!(
        metadata_size <= bytes.len(),
        "serialized metadata size exceeds file size"
    );

    Ok(SerializedHeader {
        version,
        data_offset,
        endian,
        header_size: reader.position(),
    })
}

fn read_serialized_type(
    reader: &mut Reader<'_>,
    version: u32,
    enable_type_tree: bool,
) -> Result<SerializedType> {
    let class_id = reader.read_i32()?;
    let _is_stripped_type = reader.read_bool()?;
    let _script_type_index = reader.read_i16()?;

    if class_id == 114 {
        reader.skip(16)?;
    }
    reader.skip(16)?;

    let fields = if enable_type_tree {
        read_type_tree_fields(reader, version)?
    } else {
        BTreeSet::new()
    };

    let dependency_count = reader.read_count("type dependency", MAX_TYPE_COUNT)?;
    for _ in 0..dependency_count {
        let _dependency = reader.read_i32()?;
    }

    Ok(SerializedType { class_id, fields })
}

fn read_type_tree_fields(reader: &mut Reader<'_>, version: u32) -> Result<BTreeSet<String>> {
    let node_count = reader.read_count("type tree node", MAX_TYPE_TREE_NODES)?;
    let string_buffer_size = reader.read_count("type tree string byte", MAX_TYPE_TREE_STRINGS)?;
    let mut name_offsets = Vec::with_capacity(node_count);

    for _ in 0..node_count {
        let _node_version = reader.read_u16()?;
        let _level = reader.read_u8()?;
        let _type_flags = reader.read_u8()?;
        let _type_offset = reader.read_u32()?;
        let name_offset = reader.read_u32()?;
        let _byte_size = reader.read_i32()?;
        let _index = reader.read_i32()?;
        let _meta_flag = reader.read_i32()?;
        if version >= 19 {
            let _ref_type_hash = reader.read_u64()?;
        }
        name_offsets.push(name_offset);
    }

    let strings = reader.read_exact(string_buffer_size)?;
    let mut fields = BTreeSet::new();
    for offset in name_offsets {
        if let Some(name) = resolve_type_tree_string(strings, offset) {
            fields.insert(name);
        }
    }
    Ok(fields)
}

fn resolve_type_tree_string(local_strings: &[u8], offset: u32) -> Option<String> {
    if offset & 0x8000_0000 != 0 {
        return common_string(offset & 0x7fff_ffff).map(str::to_owned);
    }

    let offset = usize::try_from(offset).ok()?;
    let tail = local_strings.get(offset..)?;
    let length = tail.iter().position(|byte| *byte == 0)?;
    std::str::from_utf8(&tail[..length])
        .ok()
        .map(str::to_owned)
}

fn common_string(offset: u32) -> Option<&'static str> {
    match offset {
        49 => Some("Array"),
        106 => Some("data"),
        161 => Some("float"),
        222 => Some("int"),
        288 => Some("m_ByteSize"),
        349 => Some("m_Enabled"),
        374 => Some("m_GameObject"),
        395 => Some("m_IsArray"),
        427 => Some("m_Name"),
        490 => Some("m_Script"),
        519 => Some("m_Type"),
        795 => Some("size"),
        840 => Some("string"),
        847 => Some("TextAsset"),
        866 => Some("Texture"),
        874 => Some("Texture2D"),
        894 => Some("TypelessData"),
        907 => Some("UInt16"),
        914 => Some("UInt32"),
        921 => Some("UInt64"),
        928 => Some("UInt8"),
        981 => Some("vector"),
        1152 => Some("FileSize"),
        _ => None,
    }
}

pub(crate) struct ObjectReader<'a> {
    reader: Reader<'a>,
}

impl<'a> ObjectReader<'a> {
    pub(crate) fn new(bytes: &'a [u8], endian: Endian) -> Self {
        Self {
            reader: Reader::new(bytes, endian),
        }
    }

    pub(crate) fn remaining(&self) -> usize {
        self.reader.remaining()
    }

    pub(crate) fn align(&mut self, alignment: usize) -> Result<()> {
        self.reader.align(alignment)
    }

    pub(crate) fn read_bool(&mut self) -> Result<bool> {
        self.reader.read_bool()
    }

    pub(crate) fn read_i32(&mut self) -> Result<i32> {
        self.reader.read_i32()
    }

    pub(crate) fn read_u32(&mut self) -> Result<u32> {
        self.reader.read_u32()
    }

    pub(crate) fn read_i64(&mut self) -> Result<i64> {
        self.reader.read_i64()
    }

    pub(crate) fn read_f32(&mut self) -> Result<f32> {
        let bits = self.reader.read_u32()?;
        Ok(f32::from_bits(bits))
    }

    pub(crate) fn read_bytes(&mut self, length: usize) -> Result<&'a [u8]> {
        self.reader.read_exact(length)
    }

    pub(crate) fn read_aligned_string(&mut self) -> Result<String> {
        let length = self.reader.read_i32()?;
        ensure!(length >= 0, "Unity string has a negative length");
        let length = usize::try_from(length).expect("non-negative i32 fits usize");
        let raw = self.reader.read_exact(length)?;
        let value = String::from_utf8_lossy(raw).into_owned();
        self.reader.align(4)?;
        Ok(value)
    }

    pub(crate) fn read_byte_array(&mut self) -> Result<Vec<u8>> {
        let length = self.reader.read_i32()?;
        ensure!(length >= 0, "Unity byte array has a negative length");
        let length = usize::try_from(length).expect("non-negative i32 fits usize");
        Ok(self.reader.read_exact(length)?.to_vec())
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    endian: Endian,
    position: usize,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], endian: Endian) -> Self {
        Self {
            bytes,
            endian,
            position: 0,
        }
    }

    fn with_position(bytes: &'a [u8], endian: Endian, position: usize) -> Result<Self> {
        ensure!(position <= bytes.len(), "reader starts past end of data");
        Ok(Self {
            bytes,
            endian,
            position,
        })
    }

    fn position(&self) -> usize {
        self.position
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.position)
    }

    fn align(&mut self, alignment: usize) -> Result<()> {
        ensure!(
            alignment.is_power_of_two(),
            "alignment must be a power of two"
        );
        let mask = alignment - 1;
        let aligned = self
            .position
            .checked_add(mask)
            .map(|position| position & !mask)
            .context("reader alignment overflow")?;
        ensure!(
            aligned <= self.bytes.len(),
            "reader alignment exceeds data size"
        );
        self.position = aligned;
        Ok(())
    }

    fn skip(&mut self, length: usize) -> Result<()> {
        let _ = self.read_exact(length)?;
        Ok(())
    }

    fn read_exact(&mut self, length: usize) -> Result<&'a [u8]> {
        let end = self
            .position
            .checked_add(length)
            .context("reader offset overflow")?;
        let value = self
            .bytes
            .get(self.position..end)
            .context("unexpected end of serialized data")?;
        self.position = end;
        Ok(value)
    }

    fn read_u8(&mut self) -> Result<u8> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_bool(&mut self) -> Result<bool> {
        Ok(self.read_u8()? != 0)
    }

    fn read_u16(&mut self) -> Result<u16> {
        let bytes: [u8; 2] = self.read_exact(2)?.try_into().expect("fixed-size read");
        Ok(match self.endian {
            Endian::Little => u16::from_le_bytes(bytes),
            Endian::Big => u16::from_be_bytes(bytes),
        })
    }

    fn read_i16(&mut self) -> Result<i16> {
        let bytes: [u8; 2] = self.read_exact(2)?.try_into().expect("fixed-size read");
        Ok(match self.endian {
            Endian::Little => i16::from_le_bytes(bytes),
            Endian::Big => i16::from_be_bytes(bytes),
        })
    }

    fn read_u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self.read_exact(4)?.try_into().expect("fixed-size read");
        Ok(match self.endian {
            Endian::Little => u32::from_le_bytes(bytes),
            Endian::Big => u32::from_be_bytes(bytes),
        })
    }

    fn read_i32(&mut self) -> Result<i32> {
        let bytes: [u8; 4] = self.read_exact(4)?.try_into().expect("fixed-size read");
        Ok(match self.endian {
            Endian::Little => i32::from_le_bytes(bytes),
            Endian::Big => i32::from_be_bytes(bytes),
        })
    }

    fn read_u64(&mut self) -> Result<u64> {
        let bytes: [u8; 8] = self.read_exact(8)?.try_into().expect("fixed-size read");
        Ok(match self.endian {
            Endian::Little => u64::from_le_bytes(bytes),
            Endian::Big => u64::from_be_bytes(bytes),
        })
    }

    fn read_i64(&mut self) -> Result<i64> {
        let bytes: [u8; 8] = self.read_exact(8)?.try_into().expect("fixed-size read");
        Ok(match self.endian {
            Endian::Little => i64::from_le_bytes(bytes),
            Endian::Big => i64::from_be_bytes(bytes),
        })
    }

    fn read_cstring(&mut self) -> Result<String> {
        let tail = self
            .bytes
            .get(self.position..)
            .context("string starts past end of serialized data")?;
        let length = tail
            .iter()
            .position(|byte| *byte == 0)
            .context("unterminated serialized string")?;
        let raw = self.read_exact(length)?;
        self.skip(1)?;
        Ok(std::str::from_utf8(raw)
            .context("serialized string is not UTF-8")?
            .to_owned())
    }

    fn read_count(&mut self, label: &str, maximum: usize) -> Result<usize> {
        let count = self.read_i32()?;
        ensure!(count >= 0, "{label} count is negative");
        let count = usize::try_from(count).expect("non-negative i32 fits usize");
        ensure!(
            count <= maximum,
            "{label} count {count} exceeds safety limit {maximum}"
        );
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modern_serialized_file_and_object_table() {
        let payload = b"texture-object";
        let bytes = build_serialized_file(payload, false);
        assert!(looks_like_serialized_file(&bytes));

        let file = SerializedFile::parse(&bytes).unwrap();
        assert_eq!(file.unity_version.major, 6000);
        assert_eq!(file.unity_version.minor, 3);
        assert_eq!(file.types.len(), 1);
        assert_eq!(file.types[0].class_id, CLASS_ID_TEXTURE2D);
        assert_eq!(file.objects.len(), 1);
        assert_eq!(file.objects[0].path_id, 123);
        assert_eq!(file.object_bytes(&file.objects[0]).unwrap(), payload);
    }

    #[test]
    fn recovers_local_type_tree_field_names() {
        let bytes = build_serialized_file(b"object", true);
        let file = SerializedFile::parse(&bytes).unwrap();
        let ty = &file.types[0];
        assert!(ty.has_field("m_MipmapLimitGroupName"));
        assert!(ty.has_field("m_PlatformBlob"));
    }

    #[test]
    fn rejects_random_bytes_as_serialized_file() {
        let bytes = vec![0x41_u8; 128];
        assert!(!looks_like_serialized_file(&bytes));
    }

    fn build_serialized_file(payload: &[u8], type_tree: bool) -> Vec<u8> {
        let mut metadata = Vec::new();
        push_cstring(&mut metadata, "6000.3.12f1");
        push_i32(&mut metadata, 13);
        metadata.push(u8::from(type_tree));
        push_i32(&mut metadata, 1);

        push_i32(&mut metadata, CLASS_ID_TEXTURE2D);
        metadata.push(0);
        metadata.extend_from_slice(&(-1_i16).to_le_bytes());
        metadata.extend_from_slice(&[0_u8; 16]);

        if type_tree {
            let mut strings = Vec::new();
            let type_offset = add_string(&mut strings, "Texture2D");
            let root_name_offset = add_string(&mut strings, "Base");
            let field_one_offset = add_string(&mut strings, "m_MipmapLimitGroupName");
            let field_two_offset = add_string(&mut strings, "m_PlatformBlob");

            push_i32(&mut metadata, 3);
            push_i32(&mut metadata, strings.len() as i32);
            push_type_tree_node(&mut metadata, 0, type_offset, root_name_offset);
            push_type_tree_node(&mut metadata, 1, type_offset, field_one_offset);
            push_type_tree_node(&mut metadata, 1, type_offset, field_two_offset);
            metadata.extend_from_slice(&strings);
        }

        push_i32(&mut metadata, 0);
        push_i32(&mut metadata, 1);
        align_vec(&mut metadata, 4);
        metadata.extend_from_slice(&123_i64.to_le_bytes());
        metadata.extend_from_slice(&0_i64.to_le_bytes());
        metadata.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        push_i32(&mut metadata, 0);

        let header_size = 48_usize;
        let data_offset = align_value(header_size + metadata.len(), 16);
        let file_size = data_offset + payload.len();
        let mut file = Vec::with_capacity(file_size);
        file.extend_from_slice(&0_u32.to_be_bytes());
        file.extend_from_slice(&0_u32.to_be_bytes());
        file.extend_from_slice(&22_u32.to_be_bytes());
        file.extend_from_slice(&0_u32.to_be_bytes());
        file.push(0);
        file.extend_from_slice(&[0_u8; 3]);
        file.extend_from_slice(&(metadata.len() as u32).to_be_bytes());
        file.extend_from_slice(&(file_size as i64).to_be_bytes());
        file.extend_from_slice(&(data_offset as i64).to_be_bytes());
        file.extend_from_slice(&0_i64.to_be_bytes());
        file.extend_from_slice(&metadata);
        file.resize(data_offset, 0);
        file.extend_from_slice(payload);
        file
    }

    fn push_type_tree_node(bytes: &mut Vec<u8>, level: u8, type_offset: u32, name_offset: u32) {
        bytes.extend_from_slice(&1_u16.to_le_bytes());
        bytes.push(level);
        bytes.push(0);
        bytes.extend_from_slice(&type_offset.to_le_bytes());
        bytes.extend_from_slice(&name_offset.to_le_bytes());
        push_i32(bytes, -1);
        push_i32(bytes, 0);
        push_i32(bytes, 0);
        bytes.extend_from_slice(&0_u64.to_le_bytes());
    }

    fn add_string(buffer: &mut Vec<u8>, value: &str) -> u32 {
        let offset = buffer.len() as u32;
        buffer.extend_from_slice(value.as_bytes());
        buffer.push(0);
        offset
    }

    fn push_i32(bytes: &mut Vec<u8>, value: i32) {
        bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn push_cstring(bytes: &mut Vec<u8>, value: &str) {
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
    }

    fn align_vec(bytes: &mut Vec<u8>, alignment: usize) {
        let target = align_value(bytes.len(), alignment);
        bytes.resize(target, 0);
    }

    fn align_value(value: usize, alignment: usize) -> usize {
        (value + alignment - 1) & !(alignment - 1)
    }
}
