use std::{
    borrow::Cow,
    cmp::Eq,
    collections::HashMap,
    hash::Hash,
    io::Cursor,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc,
    },
    thread::JoinHandle,
};

use rodio::{Decoder, OutputStream, OutputStreamHandle, Sink, Source};

pub const MAX_TRACKS: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioBytes(Arc<Cow<'static, [u8]>>);

impl AudioBytes {
    pub fn new(bytes: Cow<'static, [u8]>) -> Self {
        AudioBytes(Arc::new(bytes))
    }
}

impl AsRef<[u8]> for AudioBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

pub type AudioLibrary<K> = HashMap<K, AudioBytes>;
pub type AudioVolumes<K> = HashMap<K, f32>;

#[derive(Debug, Clone, PartialEq)]
pub struct Sound<K> {
    pub key: K,
    pub volume: f32,
    pub speed: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track<K: Clone> {
    pub key: K,
    pub volume: f32,
    pub playing: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AudioState<'a, K: Clone> {
    pub sound_volume: f32,
    pub track_volume: f32,
    pub tracks: &'a [Track<K>],
}

#[derive(Debug, Clone, PartialEq)]
struct StateUpdate<K: Clone> {
    pub sound_volume: f32,
    pub track_volume: f32,
    pub tracks: [Option<Track<K>>; MAX_TRACKS],
}

#[derive(Debug, Clone)]
enum AudioCmd<K: Clone> {
    Quit,
    Prewarm,
    State(StateUpdate<K>),
    PlaySound(Sound<K>),
    UpdateLibrary(AudioLibrary<K>, bool),
    UpdateVolumes(AudioVolumes<K>),
}

pub struct Mixer<K: Clone + Send + Eq + Hash> {
    sender: Sender<AudioCmd<K>>,
    thread: Option<JoinHandle<()>>,
    initialized: bool,
}

impl<K: Clone + Send + Eq + Hash> Drop for Mixer<K> {
    fn drop(&mut self) {
        if let Some(thread) = self.thread.take() {
            self.sender.send(AudioCmd::Quit).unwrap();
            thread.join().unwrap();
        }
    }
}

impl<K: 'static + Clone + Send + Eq + Hash> Mixer<K> {
    pub fn new(audio_library: AudioLibrary<K>, audio_volumes: Option<AudioVolumes<K>>) -> Self {
        let (sender, receiver) = mpsc::channel();

        let audio_volumes = audio_volumes.unwrap_or_default();

        // TODO: Allow dummy mixer and return None
        let thread = {
            let thread = std::thread::spawn(move || {
                let mut speaker = Speaker::new(receiver, audio_library, audio_volumes);
                while speaker.listen() {}
            });
            Some(thread)
        };

        Mixer {
            sender,
            thread,
            initialized: false,
        }
    }

    pub fn initialized(&self) -> bool {
        self.initialized
    }

    pub fn init(&mut self) {
        if !self.initialized {
            self.sender.send(AudioCmd::Prewarm).unwrap();
            self.initialized = true;
        }
    }

    pub fn update_state(&mut self, state: AudioState<K>) {
        let mut tracks = [
            None, None, None, None, None, None, None, None, None, None, None, None, None, None,
            None, None,
        ];
        for i in 0..MAX_TRACKS {
            tracks[i] = state.tracks.get(i).cloned();
        }
        let state = StateUpdate {
            sound_volume: state.sound_volume,
            track_volume: state.track_volume,
            tracks,
        };
        self.send(AudioCmd::State(state))
    }

    pub fn play_sound(&mut self, sound: Sound<K>) {
        self.send(AudioCmd::PlaySound(sound))
    }

    pub fn update_library(&mut self, library: AudioLibrary<K>, restart_tracks: bool) {
        self.send(AudioCmd::UpdateLibrary(library, restart_tracks))
    }

    pub fn update_volumes(&mut self, volumes: AudioVolumes<K>) {
        self.send(AudioCmd::UpdateVolumes(volumes))
    }

    fn send(&mut self, cmd: AudioCmd<K>) {
        assert!(self.initialized);
        if self.thread.is_some() {
            self.sender.send(cmd).unwrap();
        }
    }
}

struct Speaker<K: Clone + Send + Eq + Hash> {
    receiver: Receiver<AudioCmd<K>>,
    context: Option<(OutputStream, OutputStreamHandle)>,
    sound_volume: f32,
    track_volume: f32,
    library: AudioLibrary<K>,
    volumes: AudioVolumes<K>,
    tracks: [Option<Track<K>>; MAX_TRACKS],
    sinks: [Option<Sink>; MAX_TRACKS],
}

impl<K: Clone + Send + Eq + Hash> Speaker<K> {
    pub fn new(
        receiver: Receiver<AudioCmd<K>>,
        library: AudioLibrary<K>,
        volumes: AudioVolumes<K>,
    ) -> Self {
        Speaker {
            receiver,
            context: None,
            sound_volume: 1.0,
            track_volume: 1.0,
            library,
            volumes,
            tracks: [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
            sinks: [
                None, None, None, None, None, None, None, None, None, None, None, None, None, None,
                None, None,
            ],
        }
    }

    fn warm(&mut self) {
        if self.context.is_none() {
            let context = OutputStream::try_default().unwrap();
            self.context = Some(context);
        }
    }

    pub fn listen(&mut self) -> bool {
        let cmd = self.receiver.recv().unwrap();
        match cmd {
            AudioCmd::Quit => return false,
            AudioCmd::Prewarm => self.warm(),
            AudioCmd::State(audio_state) => {
                self.sound_volume = audio_state.sound_volume;
                self.track_volume = audio_state.track_volume;
                self.update_tracks(audio_state.tracks);
            }
            AudioCmd::PlaySound(sound) => self.play_sound(&sound),
            AudioCmd::UpdateLibrary(library, restart) => {
                self.library = library;
                if restart {
                    self.restart_all_tracks();
                }
            }
            AudioCmd::UpdateVolumes(volumes) => {
                self.volumes = volumes;
                for track in self.tracks.iter().zip(self.sinks.iter()) {
                    if let (Some(track), Some(sink)) = track {
                        let track_specific_volume = *self.volumes.get(&track.key).unwrap_or(&1.0);
                        let volume = track_specific_volume * self.track_volume * track.volume;
                        sink.set_volume(volume);
                    }
                }
            }
        }
        true
    }

    fn play_sound(&self, sound: &Sound<K>) {
        let sound_specific_volume = *self.volumes.get(&sound.key).unwrap_or(&1.0);
        let volume = sound_specific_volume * self.sound_volume * sound.volume;

        let audio_bytes = self.library.get(&sound.key);
        if let Some(audio_bytes) = audio_bytes {
            let cursor = Cursor::new(audio_bytes.clone());
            let source = Decoder::new(cursor)
                .unwrap()
                .amplify(volume)
                .speed(sound.speed)
                .convert_samples();
            if let Some((_, handle)) = self.context.as_ref() {
                handle.play_raw(source).unwrap();
            }
        }
    }

    fn update_tracks(&mut self, tracks: [Option<Track<K>>; MAX_TRACKS]) {
        for i in 0..MAX_TRACKS {
            match (&self.tracks[i], &tracks[i]) {
                (None, None) => (),
                (Some(_), None) => {
                    self.sinks[i] = None;
                }
                (None, Some(track)) => {
                    self.sinks[i] = self.create_sink(track);
                }
                (Some(old), Some(new)) => {
                    if new.key == old.key {
                        let sink = self.sinks[i].as_mut().unwrap();
                        if new.playing {
                            sink.play();
                        } else {
                            sink.pause();
                        }
                        let track_specific_volume = *self.volumes.get(&new.key).unwrap_or(&1.0);
                        let volume = track_specific_volume * self.track_volume * new.volume;
                        sink.set_volume(volume);
                    } else {
                        self.sinks[i] = self.create_sink(new);
                    }
                }
            }
        }

        self.tracks = tracks;
    }

    fn create_sink(&self, track: &Track<K>) -> Option<Sink> {
        let track_specific_volume = *self.volumes.get(&track.key).unwrap_or(&1.0);
        let volume = track_specific_volume * self.track_volume * track.volume;

        let audio_bytes = self.library.get(&track.key);
        if let Some(audio_bytes) = audio_bytes {
            let cursor = Cursor::new(audio_bytes.clone());
            let source = Decoder::new(cursor).unwrap();

            if let Some((_, handle)) = self.context.as_ref() {
                let sink = Sink::try_new(handle).unwrap();
                sink.set_volume(volume);
                if !track.playing {
                    sink.pause();
                }
                sink.append(source);
                return Some(sink);
            }
        }

        None
    }

    fn restart_all_tracks(&mut self) {
        self.sinks = [
            None, None, None, None, None, None, None, None, None, None, None, None, None, None,
            None, None,
        ];
        for (i, track) in self.tracks.iter().enumerate() {
            if let Some(track) = track {
                self.sinks[i] = self.create_sink(track);
            }
        }
    }
}
