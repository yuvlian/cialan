use std::cmp::min;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use chksum_hash_md5 as md5;
use crc32_v2::crc32;

pub const VPK_SIGNATURE: u32 = 0x55aa1234;

#[derive(Debug)]
pub struct VpkHeader {
    pub signature: u32,
    pub version: u32,
    pub tree_length: u32,
    pub embed_chunk_length: Option<u32>,
    pub chunk_hashes_length: Option<u32>,
    pub self_hashes_length: Option<u32>,
    pub signature_length: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct VpkEntry {
    pub crc32: u32,
    pub preload: Vec<u8>,
    pub archive_index: u16,
    pub archive_offset: u32,
    pub file_length: u32,
}

impl VpkEntry {
    pub fn verify(&self, data: &[u8]) -> bool {
        let checksum = crc32(0, data);
        self.crc32 == checksum
    }
}

#[derive(Debug)]
pub struct Vpk {
    pub header: VpkHeader,
    pub tree: HashMap<String, VpkEntry>,
    pub vpk_path: PathBuf,
    pub tree_checksum: Option<[u8; 16]>,
    pub chunk_hashes_checksum: Option<[u8; 16]>,
    pub file_checksum: Option<[u8; 16]>,
}

impl Vpk {
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;

        let mut header = VpkHeader {
            signature: read_u32(&mut file)?,
            version: read_u32(&mut file)?,
            tree_length: read_u32(&mut file)?,
            embed_chunk_length: None,
            chunk_hashes_length: None,
            self_hashes_length: None,
            signature_length: None,
        };

        if header.signature != VPK_SIGNATURE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid VPK signature",
            ));
        }

        let mut tree_checksum = None;
        let mut chunk_hashes_checksum = None;
        let mut file_checksum = None;

        if header.version == 2 {
            header.embed_chunk_length = Some(read_u32(&mut file)?);
            header.chunk_hashes_length = Some(read_u32(&mut file)?);
            header.self_hashes_length = Some(read_u32(&mut file)?);
            header.signature_length = Some(read_u32(&mut file)?);

            let current_pos = file.stream_position()?;
            let header_len = 4 * 7;
            let checksums_offset = header_len
                + header.tree_length as u64
                + header.embed_chunk_length.unwrap_or(0) as u64
                + header.chunk_hashes_length.unwrap_or(0) as u64;

            file.seek(SeekFrom::Start(checksums_offset))?;

            let mut tc = [0u8; 16];
            let mut chc = [0u8; 16];
            let mut fc = [0u8; 16];
            file.read_exact(&mut tc)?;
            file.read_exact(&mut chc)?;
            file.read_exact(&mut fc)?;

            tree_checksum = Some(tc);
            chunk_hashes_checksum = Some(chc);
            file_checksum = Some(fc);

            file.seek(SeekFrom::Start(current_pos))?;
        } else if header.version == 1 {
            // V1 header is already read (3 * 4 bytes)
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Unsupported VPK version",
            ));
        }

        let _header_len = if header.version == 2 { 4 * 7 } else { 4 * 3 };

        let tree_start_pos = file.stream_position()?;
        let mut tree = HashMap::new();

        loop {
            let ext = read_cstring(&mut file)?;
            if ext.is_empty() {
                break;
            }

            loop {
                let mut path_str = read_cstring(&mut file)?;
                if path_str.is_empty() {
                    break;
                }

                if path_str != " " {
                    path_str.push('/');
                } else {
                    path_str = String::new();
                }

                loop {
                    let name = read_cstring(&mut file)?;
                    if name.is_empty() {
                        break;
                    }

                    let crc32 = read_u32(&mut file)?;
                    let preload_length = read_u16(&mut file)?;
                    let archive_index = read_u16(&mut file)?;
                    let mut archive_offset = read_u32(&mut file)?;
                    let file_length = read_u32(&mut file)?;
                    let suffix = read_u16(&mut file)?;

                    if suffix != 0xffff {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Invalid VPK index suffix",
                        ));
                    }

                    if archive_index == 0x7fff {
                        archive_offset += (tree_start_pos + header.tree_length as u64) as u32;
                    }

                    let mut preload = vec![0u8; preload_length as usize];
                    file.read_exact(&mut preload)?;

                    let full_path = format!("{}{}.{}", path_str, name, ext);
                    tree.insert(
                        full_path,
                        VpkEntry {
                            crc32,
                            preload,
                            archive_index,
                            archive_offset,
                            file_length,
                        },
                    );
                }
            }
        }

        Ok(Vpk {
            header,
            tree,
            vpk_path: path,
            tree_checksum,
            chunk_hashes_checksum,
            file_checksum,
        })
    }

    pub fn verify(&self) -> io::Result<bool> {
        if self.header.version != 2 {
            return Ok(true); // V1 doesn't have VPK-wide checksums in the header
        }

        let mut file = File::open(&self.vpk_path)?;
        let header_len = if self.header.version == 2 {
            4 * 7
        } else {
            4 * 3
        };

        let mut file_hasher = md5::default();

        let mut header_buf = vec![0u8; header_len];
        file.read_exact(&mut header_buf)?;
        file_hasher.update(&header_buf);

        let mut tree_hasher = md5::default();
        let mut tree_buf = vec![0u8; self.header.tree_length as usize];
        file.read_exact(&mut tree_buf)?;
        tree_hasher.update(&tree_buf);
        file_hasher.update(&tree_buf);

        let embed_len = self.header.embed_chunk_length.unwrap_or(0) as usize;
        let mut embed_buf = vec![0u8; embed_len];
        file.read_exact(&mut embed_buf)?;
        file_hasher.update(&embed_buf);

        let chunk_hashes_len = self.header.chunk_hashes_length.unwrap_or(0) as usize;
        let mut chunk_hashes_hasher = md5::default();
        let mut chunk_hashes_buf = vec![0u8; chunk_hashes_len];
        file.read_exact(&mut chunk_hashes_buf)?;
        chunk_hashes_hasher.update(&chunk_hashes_buf);
        file_hasher.update(&chunk_hashes_buf);

        let calculated_tree_md5 = tree_hasher.digest();
        let calculated_chunk_hashes_md5 = chunk_hashes_hasher.digest();

        file_hasher.update(calculated_tree_md5.as_bytes());
        file_hasher.update(calculated_chunk_hashes_md5.as_bytes());

        let calculated_file_md5 = file_hasher.digest();

        let tc: [u8; 16] = calculated_tree_md5
            .as_bytes()
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Invalid MD5 length"))?;
        let chc: [u8; 16] = calculated_chunk_hashes_md5
            .as_bytes()
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Invalid MD5 length"))?;
        let fc: [u8; 16] = calculated_file_md5
            .as_bytes()
            .try_into()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Invalid MD5 length"))?;

        Ok(Some(tc) == self.tree_checksum
            && Some(chc) == self.chunk_hashes_checksum
            && Some(fc) == self.file_checksum)
    }

    pub fn get_file_content(&self, path: &str) -> io::Result<Vec<u8>> {
        let mut reader = self.get_reader(path)?;
        let mut content = Vec::new();
        reader.read_to_end(&mut content)?;
        Ok(content)
    }

    pub fn get_reader<'a>(&'a self, path: &str) -> io::Result<VpkFileReader<'a>> {
        let entry = self
            .tree
            .get(path)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "File not found in VPK"))?;

        let archive_file = if entry.file_length > 0 {
            let archive_path = if entry.archive_index == 0x7fff {
                self.vpk_path.clone()
            } else {
                let dir = self.vpk_path.parent().unwrap_or(Path::new("."));
                let filename = self
                    .vpk_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");

                let new_filename = filename
                    .replace("english", "")
                    .replace("dir.", &format!("{:03}.", entry.archive_index));

                dir.join(new_filename)
            };
            Some(File::open(archive_path)?)
        } else {
            None
        };

        Ok(VpkFileReader {
            _vpk: self,
            entry,
            pos: 0,
            archive_file,
        })
    }
}

pub struct VpkFileReader<'a> {
    _vpk: &'a Vpk,
    entry: &'a VpkEntry,
    pos: u64,
    archive_file: Option<File>,
}

impl<'a> Read for VpkFileReader<'a> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let total_len = self.entry.preload.len() as u64 + self.entry.file_length as u64;
        if self.pos >= total_len {
            return Ok(0);
        }

        let mut bytes_read = 0;
        let preload_len = self.entry.preload.len() as u64;

        if self.pos < preload_len {
            let available = preload_len - self.pos;
            let to_copy = min(available as usize, buf.len());
            buf[..to_copy].copy_from_slice(
                &self.entry.preload[self.pos as usize..self.pos as usize + to_copy],
            );
            self.pos += to_copy as u64;
            bytes_read += to_copy;

            if bytes_read == buf.len() || self.pos >= total_len {
                return Ok(bytes_read);
            }
        }

        if let Some(ref mut f) = self.archive_file {
            let file_pos_in_archive = self.entry.archive_offset as u64 + (self.pos - preload_len);
            f.seek(SeekFrom::Start(file_pos_in_archive))?;

            let remaining_in_file = self.entry.file_length as u64 - (self.pos - preload_len);
            let to_read = min(remaining_in_file as usize, buf.len() - bytes_read);

            let n = f.read(&mut buf[bytes_read..bytes_read + to_read])?;
            self.pos += n as u64;
            bytes_read += n;
        }

        Ok(bytes_read)
    }
}

impl<'a> Seek for VpkFileReader<'a> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let total_len = self.entry.preload.len() as u64 + self.entry.file_length as u64;
        let new_pos = match pos {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(p) => self.pos as i64 + p,
            SeekFrom::End(p) => total_len as i64 + p,
        };

        if new_pos < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid seek position",
            ));
        }

        self.pos = min(new_pos as u64, total_len);
        Ok(self.pos)
    }
}

fn read_u32<R: Read>(mut r: R) -> io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u16<R: Read>(mut r: R) -> io::Result<u16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_cstring<R: Read>(mut r: R) -> io::Result<String> {
    let mut buf = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        r.read_exact(&mut byte)?;
        if byte[0] == 0 {
            break;
        }
        buf.push(byte[0]);
    }
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
