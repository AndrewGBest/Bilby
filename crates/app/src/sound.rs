//! The quiet sounds that go with moving about.
//!
//! A tick as the highlight moves, a soft pop on a press, a lower note on the way back:
//! enough to make a remote or a tap feel answered, in the manner of a phone's own
//! interface sounds, and never more than that. They are made here, in code, from sine
//! waves that die away within a few hundredths of a second; nothing is recorded or
//! copied from anywhere. They are quiet by construction (the loudest is a twentieth
//! of full volume) and one switch in Settings turns them off.

/// What happened, for the ear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sound {
    /// The highlight moved.
    Move,
    /// Something was pressed or opened.
    Select,
    /// Stepped back.
    Back,
    /// A setting was switched.
    Toggle,
}

impl Sound {
    pub fn from_number(n: i32) -> Option<Sound> {
        match n {
            0 => Some(Sound::Move),
            1 => Some(Sound::Select),
            2 => Some(Sound::Back),
            3 => Some(Sound::Toggle),
            _ => None,
        }
    }
}

/// Samples per second the sounds are made at.
pub const RATE: u32 = 44_100;

/// The sound as samples, one channel, between -1 and 1.
pub fn samples(sound: Sound) -> Vec<f32> {
    // How long it lasts, where its pitch starts and ends, a quieter note an octave or
    // a fifth above to round it, how fast it dies away, and how loud it starts.
    let (milliseconds, from_hz, to_hz, upper, decay_ms, level) = match sound {
        Sound::Move => (16.0, 2100.0, 1700.0, 0.0, 3.5, 0.040),
        Sound::Select => (60.0, 1050.0, 1050.0, 0.35, 14.0, 0.050),
        Sound::Back => (64.0, 720.0, 540.0, 0.2, 16.0, 0.045),
        Sound::Toggle => (30.0, 1480.0, 1560.0, 0.0, 7.0, 0.042),
    };
    let count = (RATE as f32 * milliseconds / 1000.0) as usize;
    let mut phase = 0.0f32;
    (0..count)
        .map(|n| {
            let t = n as f32 / RATE as f32;
            let along = n as f32 / count as f32;
            let hz = from_hz + (to_hz - from_hz) * along;
            phase += std::f32::consts::TAU * hz / RATE as f32;
            let tone = phase.sin() + upper * (phase * 1.5).sin();
            // A millisecond to come in, so it does not click; then it dies away, and
            // the last two milliseconds are brought to nothing for the same reason.
            let attack = (t / 0.001).min(1.0);
            let decay = (-t * 1000.0 / decay_ms).exp();
            let tail = ((count - n) as f32 / (RATE as f32 * 0.002)).min(1.0);
            tone * attack * decay * tail * level
        })
        .collect()
}

/// Plays the sounds on the device's speakers, from a thread of its own so that a
/// press never waits on the sound system. With no sound device, or one that will not
/// open, it does nothing at all.
#[cfg(not(target_os = "android"))]
pub struct Player {
    send: std::sync::mpsc::Sender<Sound>,
}

#[cfg(not(target_os = "android"))]
impl Player {
    pub fn start() -> Player {
        let (send, heard) = std::sync::mpsc::channel::<Sound>();
        let _ = std::thread::Builder::new().name("sounds".into()).spawn(move || {
            // Opened on the first sound, not before: most sessions on a server or in
            // a test never make one.
            let Ok(first) = heard.recv() else { return };
            let Ok(mut stream) = rodio::OutputStreamBuilder::open_default_stream() else { return };
            stream.log_on_drop(false);
            let play = |sound: Sound| stream.mixer().add(rodio::buffer::SamplesBuffer::new(1, RATE, samples(sound)));
            play(first);
            while let Ok(sound) = heard.recv() {
                play(sound);
            }
        });
        Player { send }
    }

    pub fn play(&self, sound: Sound) {
        let _ = self.send.send(sound);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sounds_are_short_quiet_and_start_and_end_in_silence() {
        for sound in [Sound::Move, Sound::Select, Sound::Back, Sound::Toggle] {
            let wave = samples(sound);
            let seconds = wave.len() as f32 / RATE as f32;
            assert!((0.01..=0.08).contains(&seconds), "{sound:?} lasts {seconds}s");
            let peak = wave.iter().fold(0.0f32, |peak, s| peak.max(s.abs()));
            assert!((0.01..=0.07).contains(&peak), "{sound:?} peaks at {peak}: quiet, but there");
            // No click at either end.
            assert!(wave[0].abs() < 0.002 && wave[wave.len() - 1].abs() < 0.002, "{sound:?}");
            // Most of it is over in the first half: a tick, not a tone.
            let energy = |part: &[f32]| part.iter().map(|s| s * s).sum::<f32>();
            let half = wave.len() / 2;
            assert!(energy(&wave[..half]) > 4.0 * energy(&wave[half..]), "{sound:?} dies away");
            assert!(wave.iter().all(|s| s.is_finite()));
        }
        // The tick for moving is the shortest; stepping back is the lowest note.
        assert!(samples(Sound::Move).len() < samples(Sound::Toggle).len());
        assert!(samples(Sound::Toggle).len() < samples(Sound::Select).len());
        assert_eq!(Sound::from_number(2), Some(Sound::Back));
        assert_eq!(Sound::from_number(9), None);
    }
}

#[cfg(test)]
mod by_ear {
    use super::*;

    /// Not a check: writes the sounds as WAV files to listen to, and plays them through
    /// whatever sound device there is. `BILBY_SOUNDS_TO=<folder> cargo test by_ear -- --ignored`
    #[test]
    #[ignore]
    fn write_and_play_them() {
        let folder =
            std::env::var("BILBY_SOUNDS_TO").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        for (name, sound) in
            [("move", Sound::Move), ("select", Sound::Select), ("back", Sound::Back), ("toggle", Sound::Toggle)]
        {
            let wave = samples(sound);
            // A second of it: the sound four times with rests, as it would be heard in use.
            let mut data: Vec<i16> = Vec::new();
            for _ in 0..4 {
                data.extend(wave.iter().map(|s| (s * i16::MAX as f32) as i16));
                data.extend(std::iter::repeat_n(0, RATE as usize / 4));
            }
            let bytes = (data.len() * 2) as u32;
            let mut file = Vec::new();
            file.extend(b"RIFF");
            file.extend((36 + bytes).to_le_bytes());
            file.extend(b"WAVEfmt ");
            file.extend(16u32.to_le_bytes());
            file.extend(1u16.to_le_bytes());
            file.extend(1u16.to_le_bytes());
            file.extend(RATE.to_le_bytes());
            file.extend((RATE * 2).to_le_bytes());
            file.extend(2u16.to_le_bytes());
            file.extend(16u16.to_le_bytes());
            file.extend(b"data");
            file.extend(bytes.to_le_bytes());
            file.extend(data.iter().flat_map(|s| s.to_le_bytes()));
            std::fs::write(folder.join(format!("bilby-{name}.wav")), file).unwrap();
        }
        #[cfg(not(target_os = "android"))]
        {
            let player = Player::start();
            for sound in [Sound::Move, Sound::Move, Sound::Select, Sound::Toggle, Sound::Back] {
                player.play(sound);
                std::thread::sleep(std::time::Duration::from_millis(250));
            }
        }
    }
}
