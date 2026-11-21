//! Generic ESC/POS encoder.
//!
//! This builds bytes and nothing else: no device is touched here. The `Printer`
//! trait receives the finished buffer elsewhere. Only the standard command set
//! is used (no vendor extensions, no per-model branching) because there is no
//! pilot shop yet and so no specific printer to tune for.

const ESC: u8 = 0x1B;
const GS: u8 = 0x1D;
const LF: u8 = 0x0A;

/// Horizontal alignment for `ESC a`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Centre,
    Right,
}

/// Code page 437 upper half (0x80..=0xFF), in byte order. Kept as a table of
/// characters so the mapping is readable and auditable against the standard.
/// 0xFF is NBSP, as in the standard table.
const CP437_HIGH: [char; 128] = [
    'Ç', 'ü', 'é', 'â', 'ä', 'à', 'å', 'ç', 'ê', 'ë', 'è', 'ï', 'î', 'ì', 'Ä', 'Å', //
    'É', 'æ', 'Æ', 'ô', 'ö', 'ò', 'û', 'ù', 'ÿ', 'Ö', 'Ü', '¢', '£', '¥', '₧', 'ƒ', //
    'á', 'í', 'ó', 'ú', 'ñ', 'Ñ', 'ª', 'º', '¿', '⌐', '¬', '½', '¼', '¡', '«', '»', //
    '░', '▒', '▓', '│', '┤', '╡', '╢', '╖', '╕', '╣', '║', '╗', '╝', '╜', '╛', '┐', //
    '└', '┴', '┬', '├', '─', '┼', '╞', '╟', '╚', '╔', '╩', '╦', '╠', '═', '╬', '╧', //
    '╨', '╤', '╥', '╙', '╘', '╒', '╓', '╫', '╪', '┘', '┌', '█', '▄', '▌', '▐', '▀', //
    'α', 'ß', 'Γ', 'π', 'Σ', 'σ', 'µ', 'τ', 'Φ', 'Θ', 'Ω', 'δ', '∞', 'φ', 'ε', '∩', //
    '≡', '±', '≥', '≤', '⌠', '⌡', '÷', '≈', '°', '∙', '·', '√', 'ⁿ', '²', '■', '\u{00A0}',
];

/// Encode one character to CP437, or `?` when it has no place in the page.
///
/// Control characters other than newline are also replaced: product names are
/// typed by staff, and a stray ESC or GS in one would otherwise be executed by
/// the printer as a command.
fn cp437(c: char) -> u8 {
    match c {
        '\n' => LF,
        ' '..='~' => c as u8,
        _ => CP437_HIGH
            .iter()
            .position(|&h| h == c)
            .map_or(b'?', |i| 0x80 + i as u8),
    }
}

/// Chainable ESC/POS command builder.
#[derive(Default)]
pub struct Esc {
    buf: Vec<u8>,
}

impl Esc {
    pub fn new() -> Esc {
        Esc::default()
    }

    /// `ESC @`: reset the printer to its power-on state.
    pub fn init(mut self) -> Self {
        self.buf.extend([ESC, 0x40]);
        self
    }

    /// `ESC t n`: select character code page `n`.
    ///
    /// Page 0 is CP437, the page `text` encodes to. `init` only resets and does
    /// not configure, so the receipt composer is responsible for calling
    /// `code_page(0)` after `init`; otherwise a printer defaulting to another
    /// page would garble product names.
    pub fn code_page(mut self, n: u8) -> Self {
        self.buf.extend([ESC, 0x74, n]);
        self
    }

    /// Text encoded as CP437. The printer must be on code page 0; see
    /// `code_page`. Control characters are never passed through (see `cp437`).
    pub fn text(mut self, s: &str) -> Self {
        self.buf.extend(s.chars().map(cp437));
        self
    }

    /// Text followed by a line feed.
    pub fn line(self, s: &str) -> Self {
        self.text(s).text("\n")
    }

    /// `ESC a n`.
    pub fn align(mut self, a: Align) -> Self {
        let n = match a {
            Align::Left => 0,
            Align::Centre => 1,
            Align::Right => 2,
        };
        self.buf.extend([ESC, 0x61, n]);
        self
    }

    /// `ESC E n`: emphasis on or off.
    pub fn bold(mut self, on: bool) -> Self {
        self.buf.extend([ESC, 0x45, on as u8]);
        self
    }

    /// `GS ! n`: the low bits select the height multiplier, so 0x01 is height x2
    /// at width x1.
    pub fn double_height(mut self, on: bool) -> Self {
        self.buf.extend([GS, 0x21, if on { 0x01 } else { 0x00 }]);
        self
    }

    /// `ESC d n`: print and feed `n` lines.
    pub fn feed(mut self, n: u8) -> Self {
        self.buf.extend([ESC, 0x64, n]);
        self
    }

    /// `GS V 66 0`: feed to the cut position, then partial cut.
    pub fn cut(mut self) -> Self {
        self.buf.extend([GS, 0x56, 0x42, 0x00]);
        self
    }

    /// `ESC p 0 25 250`: pulse drawer pin 2; on/off times are in 2 ms units.
    pub fn kick_drawer(mut self) -> Self {
        self.buf.extend([ESC, 0x70, 0x00, 25, 250]);
        self
    }

    /// Print a QR code via the `GS ( k` family: select model 2, set module
    /// size, set error correction, store the payload, print the stored symbol.
    ///
    /// Module size 6 and error-correction level M are not constrained by the
    /// RRA specification (CIS 7.24.7 defines only the payload string); they
    /// were chosen for scanner reliability and are not mandated values.
    ///
    /// The payload is the Rwanda Revenue Authority receipt signature, so it is
    /// never truncated: a payload that cannot fit one store command is a bug in
    /// the caller and panics rather than printing a QR that fails to verify. The
    /// bound is `len + 3 <= 65535`; a receipt signature is tens of characters,
    /// so it is unreachable in practice, and a loud failure beats silently
    /// truncating a legal signature.
    pub fn qr(mut self, payload: &str) -> Self {
        const MODULE_SIZE: u8 = 6;
        const ERROR_CORRECTION_M: u8 = 49; // 48 L, 49 M, 50 Q, 51 H
        let data = payload.as_bytes();
        // pL/pH cover the payload plus the 3 bytes cn, fn, m of the command.
        let p = u16::try_from(data.len() + 3).expect("QR payload too large for one store command");
        let [p_l, p_h] = p.to_le_bytes();
        self.buf.extend([GS, 0x28, 0x6B, 4, 0, 0x31, 0x41, 0x32, 0]); // model 2
        self.buf
            .extend([GS, 0x28, 0x6B, 3, 0, 0x31, 0x43, MODULE_SIZE]);
        self.buf
            .extend([GS, 0x28, 0x6B, 3, 0, 0x31, 0x45, ERROR_CORRECTION_M]);
        self.buf
            .extend([GS, 0x28, 0x6B, p_l, p_h, 0x31, 0x50, 0x30]);
        self.buf.extend_from_slice(data);
        self.buf.extend([GS, 0x28, 0x6B, 3, 0, 0x31, 0x51, 0x30]); // print
        self
    }

    /// The accumulated bytes.
    pub fn bytes(self) -> Vec<u8> {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_esc_at() {
        assert_eq!(Esc::new().init().bytes(), vec![0x1B, 0x40]);
    }
    #[test]
    fn centre_is_esc_a_1() {
        assert_eq!(
            Esc::new().align(Align::Centre).bytes(),
            vec![0x1B, 0x61, 0x01]
        );
    }
    #[test]
    fn left_and_right_alignment() {
        assert_eq!(
            Esc::new().align(Align::Left).bytes(),
            vec![0x1B, 0x61, 0x00]
        );
        assert_eq!(
            Esc::new().align(Align::Right).bytes(),
            vec![0x1B, 0x61, 0x02]
        );
    }
    #[test]
    fn code_page_zero_is_esc_t_0() {
        assert_eq!(Esc::new().code_page(0).bytes(), vec![0x1B, 0x74, 0x00]);
    }
    #[test]
    fn bold_on_is_esc_e_1() {
        assert_eq!(Esc::new().bold(true).bytes(), vec![0x1B, 0x45, 0x01]);
        assert_eq!(Esc::new().bold(false).bytes(), vec![0x1B, 0x45, 0x00]);
    }
    #[test]
    fn double_height_is_gs_bang() {
        assert_eq!(
            Esc::new().double_height(true).bytes(),
            vec![0x1D, 0x21, 0x01]
        );
        assert_eq!(
            Esc::new().double_height(false).bytes(),
            vec![0x1D, 0x21, 0x00]
        );
    }
    #[test]
    fn feed_three_is_esc_d_3() {
        assert_eq!(Esc::new().feed(3).bytes(), vec![0x1B, 0x64, 0x03]);
    }
    #[test]
    fn cut_is_gs_v_66_0() {
        assert_eq!(Esc::new().cut().bytes(), vec![0x1D, 0x56, 0x42, 0x00]);
    }
    #[test]
    fn drawer_kick_is_esc_p_0_25_250() {
        assert_eq!(
            Esc::new().kick_drawer().bytes(),
            vec![0x1B, 0x70, 0x00, 0x19, 0xFA]
        );
    }
    #[test]
    fn line_appends_line_feed() {
        assert_eq!(Esc::new().line("Hi").bytes(), vec![b'H', b'i', 0x0A]);
    }

    /// Find the `GS ( k` store command (cn=0x31, fn=0x50) and return
    /// (pL, pH, payload bytes that follow the 3-byte sub-header).
    fn stored(bytes: &[u8]) -> (u8, u8, Vec<u8>) {
        let head = [0x1D, 0x28, 0x6B];
        let at = (0..bytes.len() - 7)
            .find(|&i| bytes[i..i + 3] == head && bytes[i + 5] == 0x31 && bytes[i + 6] == 0x50)
            .expect("store command present");
        let (p_l, p_h) = (bytes[at + 3], bytes[at + 4]);
        let len = (p_l as usize | (p_h as usize) << 8) - 3;
        (p_l, p_h, bytes[at + 8..at + 8 + len].to_vec())
    }

    #[test]
    fn a_qr_carries_its_payload_and_a_correct_length_prefix() {
        // Lengths chosen to cross the 256 boundary so pH is exercised.
        for len in [1usize, 10, 252, 253, 300, 1000] {
            let payload = "A".repeat(len);
            let out = Esc::new().qr(&payload).bytes();
            let (p_l, p_h, data) = stored(&out);
            assert_eq!(p_l as usize, (payload.len() + 3) % 256);
            assert_eq!(p_h as usize, (payload.len() + 3) / 256);
            assert_eq!(data, payload.as_bytes());
        }
    }

    #[test]
    fn a_qr_is_model_size_level_store_print_in_order() {
        let out = Esc::new().qr("RRA").bytes();
        let expected: Vec<u8> = [
            &[0x1D, 0x28, 0x6B, 4, 0, 0x31, 0x41, 0x32, 0][..],
            &[0x1D, 0x28, 0x6B, 3, 0, 0x31, 0x43, 6],
            &[0x1D, 0x28, 0x6B, 3, 0, 0x31, 0x45, 49],
            &[0x1D, 0x28, 0x6B, 6, 0, 0x31, 0x50, 0x30],
            b"RRA",
            &[0x1D, 0x28, 0x6B, 3, 0, 0x31, 0x51, 0x30],
        ]
        .concat();
        assert_eq!(out, expected);
    }

    #[test]
    fn text_is_encoded_as_cp437_with_unknown_characters_replaced() {
        let ascii = " !\"#$%&'()*+,-./0123456789:;<=>?@AZaz[\\]^_`{|}~";
        assert_eq!(Esc::new().text(ascii).bytes(), ascii.as_bytes());
        assert_eq!(
            Esc::new().text("Café").bytes(),
            vec![b'C', b'a', b'f', 0x82]
        );
        // Outside the page: CJK, emoji, and a character that would be multi-byte.
        assert_eq!(Esc::new().text("漢😀€").bytes(), vec![b'?', b'?', b'?']);
    }

    #[test]
    fn cp437_table_anchors() {
        assert_eq!(CP437_HIGH.len(), 128);
        assert_eq!(cp437('Ç'), 0x80);
        assert_eq!(cp437('ñ'), 0xA4);
        assert_eq!(cp437('░'), 0xB0);
        assert_eq!(cp437('█'), 0xDB);
        assert_eq!(cp437('±'), 0xF1);
        assert_eq!(cp437('\u{00A0}'), 0xFF);
    }

    #[test]
    fn control_characters_in_text_cannot_inject_commands() {
        assert_eq!(
            Esc::new().text("a\x1B@b\x1D").bytes(),
            vec![b'a', b'?', b'@', b'b', b'?']
        );
        assert_eq!(Esc::new().text("\x7F").bytes(), vec![b'?']);
        // Newline is the one control character that is kept.
        assert_eq!(Esc::new().text("a\nb").bytes(), vec![b'a', 0x0A, b'b']);
    }
}
