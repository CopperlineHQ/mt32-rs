// SPDX-License-Identifier: LGPL-2.1-or-later

//! The differential harness: the reference C++ engine, opened on real ROMs,
//! against this crate. Munt is the accepted yardstick for the hardware, so
//! matching it bit for bit at the native rate is the port's definition of
//! done -- these tests are the bar itself.
//!
//! ROMs are Roland's copyright and cannot be committed, so everything here
//! is `#[ignore]` and looks for a local pair under `MT32_RS_ROMS`
//! (`MT32_CONTROL.ROM` and `MT32_PCM.ROM`, as Copperline names them).

#![cfg(feature = "oracle")]

use std::ffi::c_char;
use std::os::raw::{c_int, c_uint};

extern "C" {
    fn mt32_oracle_open(
        control: *const u8,
        control_len: usize,
        pcm: *const u8,
        pcm_len: usize,
        analog_mode: c_int,
    ) -> *mut std::ffi::c_void;
    fn mt32_oracle_close(handle: *mut std::ffi::c_void);
    fn mt32_oracle_sample_rate(handle: *const std::ffi::c_void) -> c_uint;
    fn mt32_oracle_play_msg(handle: *mut std::ffi::c_void, msg: c_uint);
    fn mt32_oracle_play_sysex(handle: *mut std::ffi::c_void, sysex: *const u8, len: c_uint);
    fn mt32_oracle_render(handle: *mut std::ffi::c_void, stream: *mut i16, frames: c_uint);
    fn mt32_oracle_display(handle: *const std::ffi::c_void, buffer21: *mut c_char) -> c_int;
    fn mt32_oracle_read_memory(
        handle: *mut std::ffi::c_void,
        addr: c_uint,
        len: c_uint,
        out: *mut u8,
    );
}

/// The reference engine, opened digital-only so it renders at the native
/// 32 kHz this crate is measured against.
struct Oracle(*mut std::ffi::c_void);

// One test thread drives it at a time; the handle is not shared.
unsafe impl Send for Oracle {}

impl Oracle {
    fn open(control: &[u8], pcm: &[u8]) -> Option<Self> {
        // 0 = AnalogOutputMode_DIGITAL_ONLY: the LA32 stream itself.
        let handle = unsafe {
            mt32_oracle_open(control.as_ptr(), control.len(), pcm.as_ptr(), pcm.len(), 0)
        };
        (!handle.is_null()).then_some(Self(handle))
    }

    fn sample_rate(&self) -> u32 {
        unsafe { mt32_oracle_sample_rate(self.0) }
    }

    fn play_msg(&mut self, msg: u32) {
        unsafe { mt32_oracle_play_msg(self.0, msg) }
    }

    #[allow(dead_code)]
    fn play_sysex(&mut self, sysex: &[u8]) {
        unsafe { mt32_oracle_play_sysex(self.0, sysex.as_ptr(), sysex.len() as c_uint) }
    }

    fn render(&mut self, frames: usize) -> Vec<(i16, i16)> {
        let mut interleaved = vec![0i16; frames * 2];
        unsafe { mt32_oracle_render(self.0, interleaved.as_mut_ptr(), frames as c_uint) };
        interleaved.chunks_exact(2).map(|f| (f[0], f[1])).collect()
    }

    fn display(&self) -> (String, bool) {
        let mut buffer = [0 as c_char; 21];
        let lamp = unsafe { mt32_oracle_display(self.0, buffer.as_mut_ptr()) } != 0;
        let bytes: Vec<u8> = buffer
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as u8)
            .collect();
        (String::from_utf8_lossy(&bytes).into_owned(), lamp)
    }

    fn read_memory(&mut self, addr: u32, out: &mut [u8]) {
        unsafe { mt32_oracle_read_memory(self.0, addr, out.len() as c_uint, out.as_mut_ptr()) }
    }
}

impl Drop for Oracle {
    fn drop(&mut self) {
        unsafe { mt32_oracle_close(self.0) }
    }
}

/// The local ROM pair, if the runner has one.
fn rom_pair() -> Option<(Vec<u8>, Vec<u8>)> {
    let dir = std::path::PathBuf::from(std::env::var_os("MT32_RS_ROMS")?);
    let control = std::fs::read(dir.join("MT32_CONTROL.ROM")).ok()?;
    let pcm = std::fs::read(dir.join("MT32_PCM.ROM")).ok()?;
    Some((control, pcm))
}

/// The oracle opens, names itself on the display, runs at the native rate,
/// and renders deterministically: the same note twice gives byte-identical
/// PCM. Everything the differential tests will lean on, proven before any
/// Rust engine exists to compare.
#[test]
#[ignore = "needs a ROM pair in MT32_RS_ROMS"]
fn the_oracle_stands() {
    let Some((control, pcm)) = rom_pair() else {
        panic!("MT32_RS_ROMS is not set or the pair is unreadable");
    };

    let render_a_note = || {
        let mut oracle = Oracle::open(&control, &pcm).expect("the reference engine opens");
        assert_eq!(oracle.sample_rate(), mt32_rs::SAMPLE_RATE);
        // Skip the power-on state settling, then middle C on part 1:
        // status 0x91 in the low byte, note 0x3C, velocity 0x7F.
        oracle.render(mt32_rs::SAMPLE_RATE as usize / 4);
        oracle.play_msg(0x007F_3C91);
        let pcm_out = oracle.render(mt32_rs::SAMPLE_RATE as usize);
        let (line, _lamp) = oracle.display();
        (pcm_out, line)
    };

    let (first, line) = render_a_note();
    assert!(
        first.iter().any(|&(l, r)| l != 0 || r != 0),
        "a note makes sound"
    );
    assert!(!line.is_empty(), "the display says something: {line:?}");

    let (second, _) = render_a_note();
    assert_eq!(first, second, "the reference engine is deterministic");

    // And its memory answers: the system area starts with the master tune
    // byte, which power-on sets to 442 Hz = 0x4A.
    let mut oracle = Oracle::open(&control, &pcm).expect("opens again");
    let mut system = [0u8; 1];
    oracle.read_memory(0x10_0000, &mut system);
    assert_eq!(system[0], 0x4A, "master tune answers at its address");
}

extern "C" {
    fn mt32_oracle_identify(data: *const u8, len: usize, name32: *mut c_char) -> c_int;
}

/// What the reference engine calls one image, if it is a known full ROM.
fn oracle_identify(data: &[u8]) -> Option<String> {
    let mut name = [0 as c_char; 32];
    let full = unsafe { mt32_oracle_identify(data.as_ptr(), data.len(), name.as_mut_ptr()) };
    (full != 0).then(|| {
        let bytes: Vec<u8> = name
            .iter()
            .take_while(|&&b| b != 0)
            .map(|&b| b as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Identification agrees with the reference engine on every file in the
/// ROM directory: same short name for every full image, and nothing this
/// crate accepts that the engine does not. The directory's half- and
/// interleaved-dump files exercise the refusals.
#[test]
#[ignore = "needs a ROM directory in MT32_RS_ROMS_ALL"]
fn identification_agrees_with_the_reference_engine() {
    let Some(dir) = std::env::var_os("MT32_RS_ROMS_ALL") else {
        panic!("MT32_RS_ROMS_ALL is not set");
    };
    let mut seen = 0;
    let mut full = 0;
    for entry in std::fs::read_dir(dir)
        .expect("the directory opens")
        .flatten()
    {
        let Ok(data) = std::fs::read(entry.path()) else {
            continue;
        };
        let theirs = oracle_identify(&data);
        let ours = mt32_rs::rom::identify(&data).map(|r| r.short_name.to_string());
        assert_eq!(
            ours,
            theirs,
            "{} identified differently",
            entry.file_name().to_string_lossy()
        );
        seen += 1;
        full += usize::from(ours.is_some());
    }
    assert!(seen > 0, "no files to check");
    assert!(full > 0, "no full ROMs among them");
    println!("agreed on {seen} files, {full} of them full ROMs");
}

/// The layouts read real images correctly: every full control ROM in the
/// directory yields printable startup and error messages at the layout's
/// offsets, and its wave directory decodes in bounds against its own
/// family's sample store.
#[test]
#[ignore = "needs a ROM directory in MT32_RS_ROMS_ALL"]
fn the_layouts_read_the_real_images() {
    let Some(dir) = std::env::var_os("MT32_RS_ROMS_ALL") else {
        panic!("MT32_RS_ROMS_ALL is not set");
    };
    let mut checked = 0;
    for entry in std::fs::read_dir(dir)
        .expect("the directory opens")
        .flatten()
    {
        let Ok(data) = std::fs::read(entry.path()) else {
            continue;
        };
        let Some(info) = mt32_rs::rom::identify(&data) else {
            continue;
        };
        let Some(layout) = mt32_rs::layout::layout_for(info) else {
            continue;
        };
        let line = |offset: u16| {
            let at = usize::from(offset);
            let text = &data[at..at + 20];
            assert!(
                text.iter().all(|&b| (0x20..0x7F).contains(&b)),
                "{}: not a display line at 0x{offset:04X}: {text:?}",
                info.short_name
            );
            String::from_utf8_lossy(text).into_owned()
        };
        let startup = line(layout.startup_message);
        let error = line(layout.sysex_error_message);
        // The biggest store this control ROM's family can address; its
        // own directory must fit inside it.
        let samples = if layout.pcm_count == 256 {
            0x80000
        } else {
            0x40000
        };
        let waves = mt32_rs::pcm::waves(&data, layout, samples).expect(info.short_name);
        assert_eq!(waves.len(), usize::from(layout.pcm_count));
        println!(
            "  {:<18} startup {startup:?}  error {error:?}  {} waves",
            info.short_name,
            waves.len()
        );
        checked += 1;
    }
    assert!(checked > 0, "no control ROMs to check");
}

extern "C" {
    fn mt32_oracle_tables(
        curves559: *mut u8,
        exp512: *mut u16,
        logsin512: *mut u16,
        decay8: *mut u8,
    );
}

/// Every constant table matches the engine's, entry for entry. Needs no
/// ROMs: the tables are computed, and this is the test that catches a
/// formula mirrored at the wrong precision -- or a libm that rounds an
/// entry differently, which is worth knowing about the moment it happens.
#[test]
fn the_constant_tables_match_the_reference_engine() {
    let mut curves = [0u8; 559];
    let mut exp9 = [0u16; 512];
    let mut logsin9 = [0u16; 512];
    let mut decay = [0u8; 8];
    unsafe {
        mt32_oracle_tables(
            curves.as_mut_ptr(),
            exp9.as_mut_ptr(),
            logsin9.as_mut_ptr(),
            decay.as_mut_ptr(),
        )
    };
    let ours = mt32_rs::tables::Tables::new();
    assert_eq!(ours.level_to_amp_subtraction[..], curves[..101]);
    assert_eq!(ours.env_logarithmic_time[..], curves[101..357]);
    assert_eq!(ours.master_vol_to_amp_subtraction[..], curves[357..458]);
    assert_eq!(ours.pulse_width_100_to_255[..], curves[458..559]);
    assert_eq!(ours.exp9, exp9);
    assert_eq!(ours.logsin9, logsin9);
    assert_eq!(ours.res_amp_decay_factors, decay);
}
