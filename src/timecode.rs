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

    /// Преобразование в классический DaVinci формат `01:00:00:00`
    pub fn to_smpte(&self) -> String {
        let fps = self.fps as i64;
        let total_frames = self.frame;
        let ff = total_frames % fps;
        let total_seconds = total_frames / fps;
        let ss = total_seconds % 60;
        let total_minutes = total_seconds / 60;
        let mm = total_minutes % 60;
        let hh = total_minutes / 60;

        format!("{:02}:{:02}:{:02}:{:02}", hh, mm, ss, ff)
    }

    /// Преобразование из секунд
    pub fn from_seconds(seconds: f64, fps: u32) -> Self {
        let frame = (seconds * fps as f64).round() as i64;
        Self::new(frame, fps)
    }

    /// Получение времени в секундах
    #[allow(dead_code)]
    pub fn to_seconds(&self) -> f64 {
        self.frame as f64 / self.fps as f64
    }
}
