//#![feature(conservative_impl_trait)]

use sample;
use pitch_calc::{Letter, LetterOctave};
use std::collections::HashMap;
use bit_set::BitSet;
use core::clock::Seconds;
use num;
use num::NumCast;
use num_traits::FloatConst;
use std::iter::Iterator;
use frontend::audio::SoundEffect;
use std::f32;
use std::f64;

const CHANNELS: usize = super::CHANNELS;

#[allow(unused)]
struct Signal<S, F> where S: num::Float {
	sample_rate: S,
	frames: Box<[F]>,
}

impl<S, F> Signal<S, F> where S: num::Float {
	fn len(&self) -> usize {
		self.frames.len()
	}
}

type StereoFrame = [f32; CHANNELS as usize];
type StereoSignal = Signal<f32, StereoFrame>;

#[derive(Clone)]
struct Tone<T, S> where
	T: num::Float, S: num::Float {
	pitch: T,
	duration: Seconds,
	amplitude: S,
}

#[allow(unused)]
#[derive(Clone)]
enum Waveform<T, S> where
	T: num::Float, S: num::Float {
	Sin,
	Triangle(T),
	Harmonics(Box<[S]>, Box<[S]>),
	Square(T),
	Silence,
}

#[inline]
fn lerp_clip<T, S>(x0: T, x1: T, y0: S, y1: S, t: T) -> S
	where T: num::Float, S: num::Float {
	let v = NumCast::from((t - x0) / (x1 - x0)).unwrap();
	y0 + (y1 - y0) * S::zero().max(S::one().min(v))
}

impl<T, S> Waveform<T, S>
	where T: num::Float, S: num::Float {
	#[inline]
	fn sample(&self, phase: T) -> S where T: FloatConst {
		let phi = <S as NumCast>::from((phase + phase) * T::PI()).unwrap();
		match self {
			&Waveform::Sin => phi.sin(),
			&Waveform::Harmonics(ref hcos, ref hsin) => {
				let cos_comp =
					hcos.iter().enumerate()
						.fold(S::zero(), |sum, (i, f)| sum + *f * (phi * NumCast::from(i + 1).unwrap()).cos());
				let sin_comp =
					hsin.iter().enumerate()
						.fold(S::zero(), |sum, (i, f)| sum + *f * (phi * NumCast::from(i + 1).unwrap()).sin());
				cos_comp + sin_comp
			}
			&Waveform::Triangle(slant) => {
				if phase < slant {
					lerp_clip(T::zero(), slant, -S::one(), S::one(), phase)
				} else {
					lerp_clip(slant, T::one(), S::one(), -S::one(), phase)
				}
			}
			&Waveform::Square(duty_cycle) => {
				let s: T = (phase - duty_cycle).signum();
				NumCast::from(s).unwrap()
			}
			_ => S::zero(),
		}
	}
}

#[derive(Clone)]
struct Envelope<T, S>
	where T: num::Float, S: num::Float {
	attack: T,
	decay: T,
	sustain: S,
	release: T,
}

impl<T, S> Default for Envelope<T, S>
	where T: num::Float, S: num::Float {
	fn default() -> Self {
		Envelope {
			attack: T::zero(),
			decay: T::zero(),
			sustain: S::one(),
			release: T::zero(),
		}
	}
}

impl<T, S> Envelope<T, S>
	where T: num::Float, S: num::Float {
	#[allow(unused)]
	fn adsr(attack: T, decay: T, sustain: S, release: T) -> Self {
		Envelope {
			attack,
			decay,
			sustain,
			release,
		}
	}

	fn ramp_down(duration: T) -> Self {
		Envelope {
			release: duration,
			..Default::default()
		}
	}

	fn gain(&self, duration: T, t: T) -> S {
		if t < self.attack {
			lerp_clip(T::zero(), self.attack, S::zero(), S::one(), t)
		} else if t < self.decay {
			lerp_clip(self.attack, self.attack + self.decay, S::one(), self.sustain, t)
		} else if t < duration - self.release {
			self.sustain
		} else if t < duration {
			lerp_clip(duration - self.release, duration, self.sustain, S::zero(), t)
		} else {
			S::zero()
		}
	}
}

#[derive(Clone)]
struct Oscillator<T, S>
	where T: num::Float, S: num::Float {
	tone: Tone<T, S>,
	waveform: Waveform<T, S>,
}

#[allow(unused)]
impl<T, S> Oscillator<T, S>
	where T: num::Float + 'static, S: num::Float + sample::Sample + 'static {
	fn sin(letter_octave: LetterOctave, duration: Seconds, amplitude: S) -> Self {
		Oscillator {
			tone: Tone { pitch: NumCast::from(letter_octave.hz()).unwrap(), duration, amplitude },
			waveform: Waveform::Sin,
		}
	}

	fn square(letter_octave: LetterOctave, duration: Seconds, amplitude: S) -> Self {
		Self::pwm(letter_octave, duration, amplitude, NumCast::from(0.5).unwrap())
	}

	fn pwm(letter_octave: LetterOctave, duration: Seconds, amplitude: S, duty_cycle: T) -> Self {
		Oscillator {
			tone: Tone { pitch: NumCast::from(letter_octave.hz()).unwrap(), duration, amplitude },
			waveform: Waveform::Square(duty_cycle),
		}
	}

	fn triangle(letter_octave: LetterOctave, duration: Seconds, amplitude: S, slant: T) -> Self {
		Oscillator {
			tone: Tone { pitch: NumCast::from(letter_octave.hz()).unwrap(), duration, amplitude },
			waveform: Waveform::Triangle(slant),
		}
	}

	fn harmonics(letter_octave: LetterOctave, duration: Seconds, amplitude: S, hcos: &[S], hsin: &[S]) -> Self {
		Oscillator {
			tone: Tone { pitch: NumCast::from(letter_octave.hz()).unwrap(), duration, amplitude },
			waveform: Waveform::Harmonics(hcos.to_vec().into_boxed_slice(), hsin.to_vec().into_boxed_slice()),
		}
	}

	fn silence() -> Self {
		Oscillator {
			tone: Tone { pitch: T::one(), duration: Seconds::new(1.0f64), amplitude: S::one() },
			waveform: Waveform::Silence,
		}
	}

	fn signal_function(self, pan: S, envelope: Envelope<T, S>) -> Box<Fn(T) -> [S; CHANNELS]>
		where T: FloatConst {
		let c_pan = [S::one() - pan, pan];
		let duration: T = NumCast::from(self.duration().get()).unwrap();
		Box::new(move |t: T| {
			let t = NumCast::from(t).unwrap();
			let val = self.sample(t) * envelope.gain(duration, t);
			sample::Frame::from_fn(|channel| {
				let n = val * c_pan[channel];
				n.to_sample()
			})
		})
	}

	#[inline]
	fn sample(&self, t: T) -> S where T: FloatConst {
		self.tone.amplitude * self.waveform.sample((t * self.tone.pitch).fract())
	}

	#[inline]
	fn duration(&self) -> Seconds {
		self.tone.duration
	}

	#[inline]
	#[allow(unused)]
	fn pitch(&self) -> T {
		self.tone.pitch
	}
}

#[derive(Default, Clone)]
struct Voice {
	signal: Option<usize>,
	length: usize,
	position: usize,
}

impl Voice {
	fn new(signal_index: usize, length: usize) -> Self {
		Voice {
			signal: Some(signal_index),
			length,
			position: 0,
		}
	}

	fn remaining(&self) -> usize {
		self.length - self.position
	}

	fn advance(&mut self, l: usize) -> bool {
		self.position = usize::min(self.length, self.position + l);
		self.position >= self.length
	}
}

pub struct Multiplexer {
	#[allow(unused)]
	sample_rate: f64,
	wave_table: Vec<StereoSignal>,
	sample_map: HashMap<SoundEffect, usize>,
	voices: Vec<Voice>,
	playing_voice_index: BitSet,
	available_voice_index: Vec<usize>,
}

#[derive(Clone)]
pub struct Delay<S>
	where S: num::Float {
	time: Seconds,
	tail: Seconds,
	wet_dry: S,
	feedback: S,
}

impl<S> Default for Delay<S>
	where S: num::Float {
	fn default() -> Self {
		Delay::<S> {
			time: Seconds::new(0.25),
			tail: Seconds::new(1.0),
			wet_dry: S::one(),
			feedback: NumCast::from(0.5).unwrap(),
		}
	}
}

#[derive(Clone)]
pub struct SignalBuilder<T, S>
	where T: num::Float, S: num::Float + sample::Sample {
	oscillator: Oscillator<T, S>,
	envelope: Envelope<T, S>,
	pan: S,
	sample_rate: T,
	delay: Delay<S>,
}

impl<T, S> SignalBuilder<T, S>
	where T: num::Float + 'static, S: num::Float + sample::Sample + 'static {
	fn new() -> Self {
		SignalBuilder {
			oscillator: Oscillator::sin(LetterOctave(Letter::C, 4),
										Seconds::new(1.0),
										S::one()),
			envelope: Envelope::default(),
			sample_rate: NumCast::from(48000.0).unwrap(),
			pan: NumCast::from(0.5).unwrap(),
			delay: Delay::default(),
		}
	}

	fn from_oscillator(oscillator: Oscillator<T, S>) -> Self {
		let duration = oscillator.tone.duration;
		SignalBuilder {
			oscillator,
			envelope: Envelope::ramp_down(T::from(duration.get()).unwrap()),
			sample_rate: NumCast::from(48000.0).unwrap(),
			pan: NumCast::from(0.5).unwrap(),
			delay: Delay {
				time: duration,
				tail: duration * 4.0f64,
				..Delay::default()
			},
		}
	}

	fn with_envelope(&self, envelope: Envelope<T, S>) -> Self {
		SignalBuilder {
			envelope,
			..self.clone()
		}
	}

	fn with_envelope_ramp_down(&self) -> Self {
		self.with_envelope(Envelope::ramp_down(NumCast::from(self.oscillator.tone.duration.get()).unwrap()))
	}

	fn with_oscillator(&self, oscillator: Oscillator<T, S>) -> Self {
		SignalBuilder {
			oscillator,
			..self.clone()
		}
	}

	fn with_pan(&self, pan: S) -> Self {
		SignalBuilder {
			pan,
			..self.clone()
		}
	}

	fn with_delay(&self, delay: Delay<S>) -> Self {
		SignalBuilder {
			delay,
			..self.clone()
		}
	}

	fn with_delay_time(&self, time: Seconds) -> Self {
		self.with_delay(Delay {
			time,
			tail: time * 8.0f64,
			..self.delay.clone()
		})
	}

	fn build(&self) -> Signal<T, [S; CHANNELS]>
		where T: FloatConst {
		let duration = self.oscillator.tone.duration;
		let f: Box<Fn(T) -> [S; CHANNELS]> = self.oscillator.clone().signal_function(self.pan, self.envelope.clone());
		Signal::<T, [S; CHANNELS]>::new(self.sample_rate, duration, f)
			.with_delay(self.delay.time, self.delay.tail, self.delay.wet_dry, self.delay.feedback)
	}

	fn record(&self, wave_table: &mut Vec<Signal<T, [S; CHANNELS]>>) -> usize
		where T: FloatConst {
		let signal = self.build();
		let index = wave_table.len();
		info!("Built signal[{}] with {} samples", index, signal.len());
		wave_table.push(signal);
		index
	}
}

impl Multiplexer {
	pub fn new(sample_rate: f64, max_voices: usize) -> Multiplexer {
		let mut wave_table = Vec::new();
		let mut sample_map = HashMap::new();
		{
			let mut map_effect = |effect: SoundEffect, wave_index: usize| {
				sample_map.insert(effect, wave_index);
				info!("Assigned {:?} to signal[{}]", effect, wave_index);
			};

			map_effect(SoundEffect::Startup, SignalBuilder::from_oscillator(
				Oscillator::harmonics(LetterOctave(Letter::A, 3),
									  Seconds::new(2.0),
									  0.3f32,
									  &[0.0f32, 0.1f32, 0.0f32, 0.2f32],
									  &[0.6f32]))
				.with_envelope(Envelope::adsr(0.01, 0.5, 0.5, 0.5))
				.with_pan(0.25f32)
				.with_delay_time(Seconds::new(1.0))
				.record(&mut wave_table));

			map_effect(SoundEffect::Click(1), SignalBuilder::from_oscillator(
				Oscillator::square(LetterOctave(Letter::G, 5),
								   Seconds::new(0.1),
								   0.1f32))
				.with_pan(0.8f32)
				.with_delay_time(Seconds::new(0.25))
				.record(&mut wave_table));

			map_effect(SoundEffect::UserOption, SignalBuilder::from_oscillator(
				Oscillator::square(LetterOctave(Letter::C, 6),
								   Seconds::new(0.1),
								   0.1f32))
				.with_pan(0.6f32)
				.with_delay_time(Seconds::new(0.25))
				.record(&mut wave_table));

			map_effect(SoundEffect::Fertilised, SignalBuilder::from_oscillator(
				Oscillator::sin(LetterOctave(Letter::C, 4),
								Seconds::new(0.3),
								0.1f32))
				.with_pan(0.6f32)
				.with_delay_time(Seconds::new(0.25))
				.record(&mut wave_table));

			map_effect(SoundEffect::NewSpore, SignalBuilder::from_oscillator(
				Oscillator::harmonics(
					LetterOctave(Letter::F, 5),
					Seconds::new(0.3),
					0.1f32,
					&[0.0f32, 0.3f32, 0.0f32, 0.1f32],
					&[0.6f32]))
				.with_pan(0.3f32)
				.with_delay_time(Seconds::new(0.33))
				.record(&mut wave_table));

			map_effect(SoundEffect::NewMinion, SignalBuilder::from_oscillator(
				Oscillator::sin(LetterOctave(Letter::A, 4),
								Seconds::new(0.5),
								0.1f32))
				.with_pan(0.55f32).with_delay_time(Seconds::new(0.25))
				.record(&mut wave_table));

			map_effect(SoundEffect::DieMinion, SignalBuilder::from_oscillator(
				Oscillator::sin(LetterOctave(Letter::Eb, 3),
								Seconds::new(1.0),
								0.2f32))
				.with_pan(1f32)
				.with_delay_time(Seconds::new(0.5))
				.record(&mut wave_table));
		}

		let voices = vec![Voice::default(); max_voices];
		let playing_voice_index = BitSet::with_capacity(max_voices);
		let available_voice_index = (0..max_voices).rev().collect();

		Multiplexer {
			sample_rate,
			wave_table,
			sample_map,
			voices,
			playing_voice_index,
			available_voice_index,
		}
	}

	fn free_voice(&mut self, voice_index: usize) {
		self.voices[voice_index].signal = None;
		self.playing_voice_index.remove(voice_index);
		self.available_voice_index.push(voice_index);
	}

	fn allocate_voice(&mut self, voice: Voice) -> Option<usize> {
		let allocated = self.available_voice_index.pop();
		if let Some(voice_index) = allocated {
			self.playing_voice_index.insert(voice_index);
			self.voices[voice_index] = voice;
		}
		allocated
	}

	pub fn audio_requested(&mut self, buffer: &mut [StereoFrame]) {
		sample::slice::equilibrium(buffer);
		let mut terminated_voices = BitSet::with_capacity(self.voices.len());
		for voice_index in &self.playing_voice_index {
			let voice = self.voices[voice_index].clone();
			if let Some(signal_index) = voice.signal {
				let frames = &self.wave_table[signal_index].frames[voice.position..];
				let len = buffer.len().min(voice.remaining());
				// TODO: how do we unroll this?
				for channel in 0..CHANNELS {
					for idx in 0..len {
						buffer[idx][channel] += frames[idx][channel];
					}
				}

				if self.voices[voice_index].advance(len) {
					// returns true on EOF
					terminated_voices.insert(voice_index);
				}
			}
		}
		for voice_index in &terminated_voices {
			self.free_voice(voice_index);
			info!("Voice {} stopped", voice_index);
		}
	}

	pub fn trigger(&mut self, effect: SoundEffect) {
		if let Some(signal_index) = self.sample_map.get(&effect).map(|t| *t) {
			let signal_length = self.wave_table[signal_index].len();
			if let Some(index) = self.allocate_voice(Voice::new(signal_index, signal_length)) {
				info!("Voice {} playing, {:?}", index, effect);
			}
		}
	}
}

#[allow(unused)]
impl<S, F> Signal<S, F> where S: num::Float {
	fn new<V>(sample_rate: S, duration: Seconds, f: Box<V>) -> Signal<S, F>
		where V: Fn(S) -> F + ? Sized {
		let samples: usize = (duration.get() * sample_rate.to_f64().unwrap()).round() as usize;
		let frames = (0..samples)
			.map(|i| S::from(i).unwrap() / sample_rate)
			.map(|t| f(t)).collect::<Vec<F>>();
		Signal {
			sample_rate,
			frames: frames.into_boxed_slice(),
		}
	}

	fn duration(&self) -> Seconds {
		Seconds::new(self.sample_rate.to_f64().unwrap() * self.frames.len() as f64)
	}

	fn sample_rate(&self) -> S {
		self.sample_rate
	}
}

impl<S, T> Signal<S, [T; CHANNELS]>
	where S: num::Float, T: num::Float + sample::Sample {
	fn with_delay(self, time: Seconds, tail: Seconds, wet_dry: T, feedback: T) -> Self {
		use sample::Signal;
		let wet_ratio = wet_dry;
		let dry_ratio: T = T::one() - wet_dry;
		let source_length = self.frames.len();
		let sample_rate = self.sample_rate.to_f64().unwrap();
		let delay_length = (time.get() * sample_rate).round() as usize;
		let tail_length = (tail.get() * sample_rate).round() as usize;
		let dest_length = source_length + tail_length;
		let mut delay_buffer: Vec<[T; CHANNELS]> = sample::signal::equilibrium().take(delay_length).collect();
		let mut dest_buffer: Vec<[T; CHANNELS]> = Vec::with_capacity(source_length + tail_length);

		for i in 0..dest_length {
			let tram_ptr = i % delay_length;
			let src = *self.frames.get(i).unwrap_or(&[T::zero(), T::zero()]);
			let delay_effect = delay_buffer[tram_ptr];
			let wet: [T; CHANNELS] = sample::Frame::from_fn(move |channel| { wet_ratio * src[channel] + delay_effect[channel] });
			dest_buffer.push(sample::Frame::from_fn(move |channel| { dry_ratio * src[channel] + wet[channel] }));
			delay_buffer[tram_ptr] = sample::Frame::from_fn(move |channel| { feedback * wet[CHANNELS - 1 - channel] }); // ping-pong
		}
		self::Signal {
			sample_rate: self.sample_rate,
			frames: dest_buffer.into_boxed_slice(),
		}
	}
}
