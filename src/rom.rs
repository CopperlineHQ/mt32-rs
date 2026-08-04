// SPDX-License-Identifier: LGPL-2.1-or-later

//! ROM images: what they are, and whether a file is one.
//!
//! The engine identifies a ROM by its size and SHA-1 rather than its
//! filename, so a mislabelled or truncated file is refused before it can
//! open a synth that sounds subtly wrong. Phase 1 fills this module in with
//! the known-image table and the control/PCM structural readers; for now it
//! carries only the sizes every later phase agrees on.

/// A full control ROM image: 64 KiB for the first-generation units, 128 KiB
/// for the second, whose upper half holds the demo songs the engine never
/// reads.
pub const CONTROL_ROM_SIZES: [usize; 2] = [64 * 1024, 128 * 1024];

/// A full PCM ROM image: 512 KiB on the MT-32, 1 MiB on the CM-32L family.
pub const PCM_ROM_SIZES: [usize; 2] = [512 * 1024, 1024 * 1024];
