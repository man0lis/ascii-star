use pitch_calc::*;

fn do_autocorrelation_with_freq(samples: &[f32], sample_rate: f64, freq: f64) -> f64 {
    let samples_per_period = (sample_rate / freq).round() as usize;
    let correlating_sample_iter = samples.iter().skip(samples_per_period);
    let sample_zipped_iter = samples.iter().zip(correlating_sample_iter);
    let accum_dist = sample_zipped_iter.fold(0.0, |acc, (x, y)| acc + (x - y).abs());
    1.0 - accum_dist as f64 / samples.len() as f64
}

fn get_note_wieghts(samples: &[f32], sample_rate: f64) -> Vec<(LetterOctave, f64)> {
    let first_tone = LetterOctave(Letter::C, 2);
    let last_tone = LetterOctave(Letter::A, 5);

    let first_semitone = first_tone.to_step().step() as i32;
    let last_semitone = last_tone.to_step().step() as i32;

    (first_semitone..last_semitone)
        .map(|step| {
            let step_float = step as f32;
            (
                Step(step_float).to_letter_octave(),
                do_autocorrelation_with_freq(
                    samples,
                    sample_rate,
                    Step(step_float).to_hz().hz() as f64,
                ),
            )
        })
        .collect::<Vec<_>>()
}

pub fn get_dominant_note(samples: &[f32], sample_rate: f64) -> LetterOctave {
    get_note_wieghts(samples, sample_rate)
        .iter()
        .fold(
            (LetterOctave(Letter::C, 2), -1.0),
            |(old_note, old_max_wight), &(note, weight)| if weight > old_max_wight {
                (note, weight)
            } else {
                (old_note, old_max_wight)
            },
        )
        .0
}

pub fn get_max_amplitude(samples: &[f32]) -> f32 {
    samples.iter().map(|x| x.abs()).fold(0.0, f32::max)
}
