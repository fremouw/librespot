use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::AudioFilter;
use super::{Mixer, MixerConfig};

#[derive(Clone)]
pub struct DummyMixer {
    volume: Arc<AtomicUsize>,
}

impl Mixer for DummyMixer {
    fn open(_: Option<MixerConfig>) -> DummyMixer {
        DummyMixer {
            volume: Arc::new(AtomicUsize::new(0xFFFF)),
        }
    }
    fn start(&self) {}
    fn stop(&self) {}
    fn volume(&self) -> u16 {
        self.volume.load(Ordering::Relaxed) as u16
    }
    fn set_volume(&self, volume: u16) {
        self.volume.store(volume as usize, Ordering::Relaxed);
    }
    fn get_audio_filter(&self) -> Option<Box<dyn AudioFilter + Send>> {
        None
    }
}
