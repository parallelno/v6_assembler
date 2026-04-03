/// Character encoding support for .text directives

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingType {
    Ascii,
    ScreencodeCommodore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncodingCase {
    Mixed,
    Lower,
    Upper,
}

#[derive(Debug, Clone, Copy)]
pub struct Encoding {
    pub encoding_type: EncodingType,
    pub case: EncodingCase,
}

impl Default for Encoding {
    fn default() -> Self {
        Self {
            encoding_type: EncodingType::Ascii,
            case: EncodingCase::Mixed,
        }
    }
}

impl Encoding {
    pub fn encode_char(&self, ch: char) -> u8 {
        match self.encoding_type {
            EncodingType::Ascii => {
                let c = ch as u8;
                match self.case {
                    EncodingCase::Mixed => c,
                    EncodingCase::Upper => {
                        if c >= b'a' && c <= b'z' {
                            c - 32
                        } else {
                            c
                        }
                    }
                    EncodingCase::Lower => {
                        if c >= b'A' && c <= b'Z' {
                            c + 32
                        } else {
                            c
                        }
                    }
                }
            }
            EncodingType::ScreencodeCommodore => {
                screencode_commodore(ch, self.case)
            }
        }
    }

    pub fn encode_string(&self, s: &str) -> Vec<u8> {
        s.chars().map(|c| self.encode_char(c)).collect()
    }
}

fn screencode_commodore(ch: char, case: EncodingCase) -> u8 {
    let c = ch as u8;
    match case {
        EncodingCase::Mixed | EncodingCase::Upper => {
            match c {
                b'@' => 0x00,
                b'A'..=b'Z' => c - b'A' + 1,
                b'a'..=b'z' => c - b'a' + 1,
                b'[' => 0x1B,
                b']' => 0x1D,
                b' ' => 0x20,
                b'!'..=b'?' => c - b' ',
                _ => c,
            }
        }
        EncodingCase::Lower => {
            match c {
                b'@' => 0x00,
                b'a'..=b'z' => c - b'a' + 1,
                b'A'..=b'Z' => c - b'A' + 0x41,
                b' ' => 0x20,
                b'!'..=b'?' => c - b' ',
                _ => c,
            }
        }
    }
}

impl EncodingType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "ASCII" => Some(EncodingType::Ascii),
            "SCREENCODECOMMODORE" => Some(EncodingType::ScreencodeCommodore),
            _ => None,
        }
    }
}

impl EncodingCase {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "MIXED" => Some(EncodingCase::Mixed),
            "LOWER" => Some(EncodingCase::Lower),
            "UPPER" => Some(EncodingCase::Upper),
            _ => None,
        }
    }
}
