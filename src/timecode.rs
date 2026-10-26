/// SMPTE-таймкод в виде стековой строки без аллокаций в куче (0 байт в Heap)
#[derive(Clone, Copy)]
pub struct SmpteString([u8; 11]);

impl SmpteString {
    #[inline]
    pub fn as_str(&self) -> &str {
        // SAFETY: буфер гарантированно состоит из корректных ASCII-цифр и двоеточий "00:00:00:00"
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }
}

impl std::ops::Deref for SmpteString {
    type Target = str;
    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl std::fmt::Display for SmpteString {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Утилиты работы с SMPTE таймкодом (HH:MM:SS:FF)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timecode {
    pub frame: i64,
    pub fps: u32,
}

impl Timecode {
    pub fn new(frame: i64, fps: u32) -> Self {
        Self {
            frame: frame.max(0),
            fps: if fps == 0 { 24 } else { fps },
        }
    }

    /// Преобразование в классический DaVinci формат `01:00:00:00` без единой аллокации (на стеке)
    #[inline]
    pub fn to_smpte_str(&self) -> SmpteString {
        let fps = self.fps as i64;
        let total_frames = self.frame;
        let ff = (total_frames % fps) as u8;
        let total_seconds = total_frames / fps;
        let ss = (total_seconds % 60) as u8;
        let total_minutes = total_seconds / 60;
        let mm = (total_minutes % 60) as u8;
        let hh = ((total_minutes / 60) % 100) as u8;

        let buf = [
            b'0' + hh / 10,
            b'0' + hh % 10,
            b':',
            b'0' + mm / 10,
            b'0' + mm % 10,
            b':',
            b'0' + ss / 10,
            b'0' + ss % 10,
            b':',
            b'0' + ff / 10,
            b'0' + ff % 10,
        ];
        SmpteString(buf)
    }

    /// Преобразование в классический DaVinci формат `01:00:00:00`
    pub fn to_smpte(&self) -> String {
        self.to_smpte_str().as_str().to_string()
    }

    /// Преобразование из секунд
    pub fn from_seconds(seconds: f64, fps: u32) -> Self {
        let frame = (seconds * fps as f64).round() as i64;
        Self::new(frame, fps)
    }
}
