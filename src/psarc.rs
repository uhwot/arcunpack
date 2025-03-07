use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
use binrw::BinRead;
use anyhow::{Context, Result};
use md5::{Digest, Md5};

fn to_u64(bytes: [u8; 5]) -> u64 {
    let mut array = [0u8; 8];
    array[3..].copy_from_slice(&bytes);
    u64::from_be_bytes(array)
}

#[derive(Debug, BinRead, Copy, Clone)]
struct Entry {
    name_md5: [u8; 16],
    block_offset: u32,
    #[br(map = |x: [u8; 5]| to_u64(x))]
    uncompressed_size: u64,
    #[br(map = |x: [u8; 5]| to_u64(x))]
    file_offset: u64,
}

#[derive(Debug, BinRead)]
#[br(magic = b"PSAR", big)]
pub struct Header {
    major_version: u16,
    minor_version: u16,
    compression_type: [u8; 4],
    toc_length: u32,
    toc_entry_size: u32,
    toc_entry_count: u32,
    default_block_size: u32,
    archive_flags: u32,

    #[br(count = toc_entry_count)]
    toc_entries: Vec<Entry>,
    #[br(count = (toc_entries[0].file_offset as usize - 28 - (30 * toc_entry_count as usize)) / 2)]
    block_sizes: Vec<u16>,
}

pub struct PsArc {
    pub header: Header,
    file: File,
}

struct FileIter<'a> {
    file: &'a mut File,
    block_sizes: &'a [u16],
    default_block_size: u32,

    block_offset: u32,
    total_size: u64,
    current_size: u64,

    compressed: bool
}

impl<'a> FileIter<'a> {
    fn new(file: &'a mut File, entry: &Entry, block_sizes: &'a [u16], default_block_size: u32, compressed: bool) -> Result<Self> {
        file.seek(SeekFrom::Start(entry.file_offset))?;
        Ok(Self {
            file,
            block_sizes,
            default_block_size,

            block_offset: entry.block_offset,
            total_size: entry.uncompressed_size,
            current_size: 0,

            compressed
        })
    }
}

impl Iterator for FileIter<'_> {
    type Item = Vec<u8>;
    fn next(&mut self) -> Option<Self::Item> {
        //println!("total size: {}", self.total_size);
        if self.current_size < self.total_size {
            let mut block_size = self.block_sizes[self.block_offset as usize] as u32;
            self.block_offset += 1;
            if block_size == 0 {
                block_size = self.default_block_size;
            }

            //let position = self.file.stream_position().unwrap();
            //println!("pos: {position}, block size: {block_size}");
            let mut block = vec![0; block_size as usize];
            self.file.read_exact(&mut block).unwrap();

            if (self.current_size + block_size as u64) == self.total_size || !self.compressed {
                self.current_size += block_size as u64;
                return Some(block);
            }

            let data = lzokay_native::decompress_all(&block, None).unwrap();
            self.current_size += data.len() as u64;
            Some(data)
        } else {
            None
        }
    }
}

impl PsArc {
    pub fn new(mut file: File) -> Result<Self> {
        Ok(Self {
            header: Header::read(&mut file).context("Failed to read PSARC")?,
            file,
        })
    }

    fn unpack_to_vec(&mut self, entry: &Entry) -> Result<Vec<u8>> {
        let mut data = vec![0u8; entry.uncompressed_size as usize];
        let iter = FileIter::new(
            &mut self.file,
            entry,
            &self.header.block_sizes,
            self.header.default_block_size,
            true
        )?;
        let mut current_size = 0;
        for block in iter {
            data[current_size..current_size + block.len()].copy_from_slice(&block);
            current_size += block.len();
        }
        Ok(data)
    }

    fn get_file_path_map(&mut self) -> Result<HashMap<[u8; 16], String>> {
        let manifest = self.header.toc_entries[0];
        let manifest = self.unpack_to_vec(&manifest)?;
        let manifest = String::from_utf8(manifest)?;

        let mut map = HashMap::new();
        for line in manifest.lines() {
            let md5 = Md5::digest(line.to_uppercase().as_bytes());
            let md5 = *md5.first_chunk::<16>().unwrap();
            map.insert(md5, line.to_string());
        }

        Ok(map)
    }

    fn unpack_to_file(&mut self, file: &mut File, entry: &Entry, compressed: bool) -> Result<()> {
        let iter = FileIter::new(
            &mut self.file,
            entry,
            &self.header.block_sizes,
            self.header.default_block_size,
            compressed
        )?;
        for block in iter {
            file.write_all(&block)?;
        }
        Ok(())
    }

    pub fn unpack(&mut self) -> Result<()> {
        let file_path_map = self.get_file_path_map()?;

        for entry in self.header.toc_entries[1..].to_owned().iter() {
            let path = file_path_map.get(&entry.name_md5).context("Couldn't find file path")?;
            println!("Unpacking {path}...");

            let compressed = (!path.ends_with(".png") && !path.ends_with(".at3") && !path.ends_with(".bnk"))
                || path.ends_with("snd0_1.at3");

            let path_dir = Path::new(&path).parent().context("Couldn't get parent directory")?;
            let path_dir = format!("unpacked{}", path_dir.display());
            fs::create_dir_all(path_dir).context("Couldn't create parent directory")?;

            // TODO: this is ass
            let path = format!("unpacked{path}");
            let mut file = File::create(path).context("Failed to create file")?;

            self.unpack_to_file(&mut file, entry, compressed)?;
        }
        Ok(())
    }
}