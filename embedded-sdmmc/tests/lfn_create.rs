//! Tests for creating files with long file names (LFN creation support).

use core::ops::ControlFlow;

use embedded_sdmmc::{LfnBuffer, Mode, VolumeIdx, VolumeManager};

mod utils;

type Vm = VolumeManager<utils::RamDisk<Vec<u8>>, utils::TestTimeSource, 4, 2, 1>;

/// Open the given partition and its root directory on a fresh in-memory disk.
fn fresh_mgr(
    partition: VolumeIdx,
) -> (
    Vm,
    embedded_sdmmc::RawVolume,
    embedded_sdmmc::RawDirectory,
) {
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let volume_mgr: Vm = VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr.open_raw_volume(partition).expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");
    (volume_mgr, volume, root_dir)
}

/// Look for `name` in an LFN directory listing, returning its size if found.
fn lfn_listing_size(volume_mgr: &Vm, root_dir: embedded_sdmmc::RawDirectory, name: &str) -> Option<u32> {
    let mut storage = [0u8; 512];
    let mut lfn_buffer = LfnBuffer::new(&mut storage);
    let mut hit: Option<u32> = None;
    volume_mgr
        .iterate_dir_lfn(root_dir, &mut lfn_buffer, |entry, lfn| {
            if lfn == Some(name) {
                hit = Some(entry.size);
                ControlFlow::Break(())
            } else {
                ControlFlow::Continue(())
            }
        })
        .expect("iterate");
    hit
}

/// Create a file with a (non-ASCII, non-8.3) long name, then reopen it by that
/// long name and read the data back.
fn check_create_and_reopen(partition: VolumeIdx) {
    let (volume_mgr, volume, root_dir) = fresh_mgr(partition);

    let long_name = "三体全书.txt";
    let data = b"Long file name content\n";

    let f = volume_mgr
        .open_long_name_file_in_dir(root_dir, long_name, Mode::ReadWriteCreate)
        .expect("create LFN file");
    volume_mgr.write(f, data).expect("write data");
    volume_mgr.close_file(f).expect("close file");

    // The long name must appear in a directory listing with the right size.
    assert_eq!(lfn_listing_size(&volume_mgr, root_dir, long_name), Some(data.len() as u32));

    // Re-open read-only by the long name (proves the LFN entries round-trip).
    let f2 = volume_mgr
        .open_long_name_file_in_dir(root_dir, long_name, Mode::ReadOnly)
        .expect("reopen by LFN");
    assert_eq!(volume_mgr.file_length(f2).unwrap(), data.len() as u32);
    let mut buf = vec![0u8; data.len()];
    let n = volume_mgr.read(f2, &mut buf).expect("read back");
    assert_eq!(n, data.len());
    assert_eq!(&buf[..n], data);
    volume_mgr.close_file(f2).expect("close");

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}

#[test]
fn create_and_reopen_lfn_fat32() {
    check_create_and_reopen(VolumeIdx(1));
}

#[test]
fn create_and_reopen_hyphenated_chinese_lfn() {
    // 中文名 + 连字符 + 中文（如 "思源宋-细.ttf"），验证 find_by_lfn 对混合名工作
    let time_source = utils::make_time_source();
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    let volume_mgr: Vm = VolumeManager::new_with_limits(disk, time_source, 0xAA00_0000);
    let volume = volume_mgr.open_raw_volume(VolumeIdx(1)).expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");

    let long_name = "思源宋-细.ttf";
    let f = volume_mgr
        .open_long_name_file_in_dir(root_dir, long_name, Mode::ReadWriteCreate)
        .expect("create hyphenated Chinese LFN");
    volume_mgr.write(f, b"x").expect("write");
    volume_mgr.close_file(f).expect("close");

    // find_by_lfn 必须能找回
    let f2 = volume_mgr
        .open_long_name_file_in_dir(root_dir, long_name, Mode::ReadOnly)
        .expect("reopen hyphenated Chinese LFN by find_by_lfn");
    volume_mgr.close_file(f2).expect("close");

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}

#[test]
fn create_and_reopen_lfn_fat16() {
    check_create_and_reopen(VolumeIdx(0));
}

#[test]
fn append_to_existing_lfn() {
    // ReadWriteCreateOrAppend on an existing LFN file should append, not error.
    let (volume_mgr, volume, root_dir) = fresh_mgr(VolumeIdx(1));
    let long_name = "AppendTestLongName.dat";

    let f = volume_mgr
        .open_long_name_file_in_dir(root_dir, long_name, Mode::ReadWriteCreate)
        .expect("create");
    volume_mgr.write(f, b"first").expect("write first");
    volume_mgr.close_file(f).expect("close");

    let f = volume_mgr
        .open_long_name_file_in_dir(root_dir, long_name, Mode::ReadWriteCreateOrAppend)
        .expect("append-open existing LFN");
    volume_mgr.write(f, b"-second").expect("write second");
    volume_mgr.close_file(f).expect("close");

    assert_eq!(
        lfn_listing_size(&volume_mgr, root_dir, long_name),
        Some("first-second".len() as u32)
    );

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}

#[test]
fn lfn_collision_gets_unique_short_names() {
    // Two CJK names mangle to the same 8.3 base; both must still be
    // independently addressable by their long names.
    let (volume_mgr, volume, root_dir) = fresh_mgr(VolumeIdx(1));
    let names = ["三体.txt", "三国.txt"];

    for name in names {
        let f = volume_mgr
            .open_long_name_file_in_dir(root_dir, name, Mode::ReadWriteCreate)
            .expect("create");
        volume_mgr.close_file(f).expect("close");
    }

    for name in names {
        assert!(
            lfn_listing_size(&volume_mgr, root_dir, name).is_some(),
            "{name} not found by long name"
        );
    }

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}

#[test]
fn create_existing_lfn_with_create_mode_errors() {
    // Plain ReadWriteCreate on an existing LFN file must report AlreadyExists.
    let (volume_mgr, volume, root_dir) = fresh_mgr(VolumeIdx(1));
    let long_name = "DuplicateLongName.bin";

    let f = volume_mgr
        .open_long_name_file_in_dir(root_dir, long_name, Mode::ReadWriteCreate)
        .expect("first create");
    volume_mgr.close_file(f).expect("close");

    let err = volume_mgr
        .open_long_name_file_in_dir(root_dir, long_name, Mode::ReadWriteCreate)
        .unwrap_err();
    use embedded_sdmmc::Error;
    assert!(matches!(err, Error::FileAlreadyExists), "got {err:?}");

    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}

/// Build the raw 32-byte directory entries for `name` (one or more LFN slots
/// in on-disk order — highest sequence first — followed by the SFN entry),
/// in standard FAT format with the LFN checksum derived from `sfn_name`.
fn build_lfn_sfn_entries(name: &str, sfn_name: [u8; 11]) -> std::vec::Vec<[u8; 32]> {
    let units: std::vec::Vec<u16> = name.encode_utf16().collect();
    let num_slots = (units.len() + 12) / 13;
    let mut csum = 0u8;
    for &b in sfn_name.iter() {
        csum = csum.rotate_right(1).wrapping_add(b);
    }
    let mut entries = std::vec::Vec::new();
    for k in (1..=num_slots).rev() {
        let start = (k - 1) * 13;
        let end = (k * 13).min(units.len());
        let mut buf = [0xFFFFu16; 13];
        for (i, &u) in units[start..end].iter().enumerate() {
            buf[i] = u;
        }
        if end - start < 13 {
            buf[end - start] = 0x0000;
        }
        let seq_byte = if k == num_slots { 0x40 | (k as u8) } else { k as u8 };
        let mut slot = [0u8; 32];
        slot[0] = seq_byte;
        slot[11] = 0x0F;
        slot[13] = csum;
        for i in 0..5 {
            slot[1 + 2 * i] = (buf[i] & 0xFF) as u8;
            slot[2 + 2 * i] = (buf[i] >> 8) as u8;
        }
        for i in 0..6 {
            slot[14 + 2 * i] = (buf[5 + i] & 0xFF) as u8;
            slot[15 + 2 * i] = (buf[5 + i] >> 8) as u8;
        }
        for i in 0..2 {
            slot[28 + 2 * i] = (buf[11 + i] & 0xFF) as u8;
            slot[29 + 2 * i] = (buf[11 + i] >> 8) as u8;
        }
        entries.push(slot);
    }
    let mut sfn = [0u8; 32];
    sfn[0..11].copy_from_slice(&sfn_name);
    sfn[11] = 0x20;
    entries.push(sfn);
    entries
}

/// Regression for `find_by_lfn` getting stuck in the `Scanning` state.
///
/// A multi-slot LFN whose name shares a suffix (e.g. ".ttf") with the target
/// partially matches the target's suffix, moving the matcher into `Scanning`.
/// On the next slot's mismatch the engine must reset to `Waiting` or it skips
/// every remaining entry — so the target (and anything after the sibling) is
/// reported `NotFound`. This reproduces the real failure: opening a
/// Chinese-named font failed whenever it sorted after another multi-slot
/// `*.ttf` entry in the same directory.
#[test]
fn find_lfn_after_sibling_with_shared_suffix() {
    use embedded_sdmmc::{Block, BlockDevice, BlockIdx};
    let disk = utils::make_block_device(utils::DISK_SOURCE).unwrap();
    const ROOT_DIR_BLOCK: u32 = 265760; // FAT32 root dir sector on this image

    // Inject a multi-slot sibling (ends in ".ttf") BEFORE the target, so the
    // matcher hits it first and partially matches the target's ".ttf" suffix.
    let distractor = build_lfn_sfn_entries("aaaaaaaaaaaaa.ttf", *b"AAAAAA~1TTF");
    let target = build_lfn_sfn_entries("目标.ttf", *b"_______1TTF");

    let mut block = Block::new();
    let need = distractor.len() + target.len();
    // The FAT32 root dir spans several blocks (cluster 2, 8 blocks). The first
    // block holds the existing files; inject into a later block where entries
    // are free, so the sibling lands before the target in traversal order.
    let mut inject_block = ROOT_DIR_BLOCK;
    let mut start = None;
    for blk_off in 0..4 {
        let b = ROOT_DIR_BLOCK + blk_off;
        BlockDevice::read(&disk, core::slice::from_mut(&mut block), BlockIdx(b))
            .expect("read root dir block");
        for i in 0..=(16 - need) {
            if (0..need).all(|j| {
                let bb = block.contents[(i + j) * 32];
                bb == 0x00 || bb == 0xE5
            }) {
                start = Some(i);
                break;
            }
        }
        if start.is_some() {
            inject_block = b;
            break;
        }
    }
    let start = start.expect("found enough consecutive free root-dir entries");
    let mut p = start;
    for e in distractor.iter().chain(target.iter()) {
        block.contents[p * 32..(p + 1) * 32].copy_from_slice(e);
        p += 1;
    }
    BlockDevice::write(&disk, core::slice::from_ref(&block), BlockIdx(inject_block))
        .expect("write root dir");

    let volume_mgr: Vm =
        VolumeManager::new_with_limits(disk, utils::make_time_source(), 0xAA00_0000);
    let volume = volume_mgr.open_raw_volume(VolumeIdx(1)).expect("open volume");
    let root_dir = volume_mgr.open_root_dir(volume).expect("open root dir");
    let f = volume_mgr
        .open_long_name_file_in_dir(root_dir, "目标.ttf", Mode::ReadOnly)
        .expect("must find target despite a sibling sharing the '.ttf' suffix");
    volume_mgr.close_file(f).expect("close");
    volume_mgr.close_dir(root_dir).expect("close dir");
    volume_mgr.close_volume(volume).expect("close volume");
}
