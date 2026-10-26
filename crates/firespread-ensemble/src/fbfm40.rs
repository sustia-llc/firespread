//! LANDFIRE FBFM40 raster codes.

/// The 40 burnable FBFM40 codes and the Scott and Burgan (2005) model each
/// one names, in table order: GR 101-109, GS 121-124, SH 141-149, TU 161-165,
/// TL 181-189, SB 201-204.
pub const BURNABLE_FBFM40: [(u16, &str); 40] = [
    (101, "GR1"),
    (102, "GR2"),
    (103, "GR3"),
    (104, "GR4"),
    (105, "GR5"),
    (106, "GR6"),
    (107, "GR7"),
    (108, "GR8"),
    (109, "GR9"),
    (121, "GS1"),
    (122, "GS2"),
    (123, "GS3"),
    (124, "GS4"),
    (141, "SH1"),
    (142, "SH2"),
    (143, "SH3"),
    (144, "SH4"),
    (145, "SH5"),
    (146, "SH6"),
    (147, "SH7"),
    (148, "SH8"),
    (149, "SH9"),
    (161, "TU1"),
    (162, "TU2"),
    (163, "TU3"),
    (164, "TU4"),
    (165, "TU5"),
    (181, "TL1"),
    (182, "TL2"),
    (183, "TL3"),
    (184, "TL4"),
    (185, "TL5"),
    (186, "TL6"),
    (187, "TL7"),
    (188, "TL8"),
    (189, "TL9"),
    (201, "SB1"),
    (202, "SB2"),
    (203, "SB3"),
    (204, "SB4"),
];

/// What an FBFM40 code denotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fbfm40 {
    /// A burnable model: its index in [`BURNABLE_FBFM40`].
    Burnable(usize),
    /// A non-burnable class, codes 91-99.
    NonBurnable,
}

/// Classifies an FBFM40 code: one of [`BURNABLE_FBFM40`], a non-burnable
/// code 91-99, or `None` for any other code.
#[must_use]
pub fn classify_fbfm40(code: u16) -> Option<Fbfm40> {
    if (91..=99).contains(&code) {
        return Some(Fbfm40::NonBurnable);
    }
    BURNABLE_FBFM40
        .iter()
        .position(|&(c, _)| c == code)
        .map(Fbfm40::Burnable)
}
