#![recursion_limit = "1024"]
#[macro_use]
extern crate error_chain;

extern crate clap;
extern crate colored;
extern crate env_logger;
extern crate gstreamer as gst;
#[macro_use]
extern crate log;
extern crate termion;
extern crate ultrastar_txt;

use std::io::{stdout, Write};
use std::path::Path;
use gst::MessageView;
use gst::prelude::*;
use colored::*;
use clap::{App, Arg};

mod errors {
    error_chain!{}
}
use errors::*;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");
const AUTHOR: &'static str = env!("CARGO_PKG_AUTHORS");

struct CustomData {
    playbin: gst::Element,    // Our one and only element
    playing: bool,            // Are we in the PLAYING state?
    terminate: bool,          // Should we terminate execution?
    duration: gst::ClockTime, // How long does this media last, in nanoseconds
}

fn main() {
    if let Err(ref e) = run() {
        use std::io::Write;
        let stderr = &mut ::std::io::stderr();
        let errmsg = "Error writing to stderr";

        writeln!(stderr, "error: {}", e).expect(errmsg);

        for e in e.iter().skip(1) {
            writeln!(stderr, "caused by: {}", e).expect(errmsg);
        }

        // The backtrace is not always generated. Try to run this example
        // with `RUST_BACKTRACE=1`.
        if let Some(backtrace) = e.backtrace() {
            writeln!(stderr, "backtrace: {:?}", backtrace).expect(errmsg);
        }

        ::std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let _ = env_logger::init();

    // manage command line arguments using clap
    let matches = App::new("usrs-cli")
        .version(VERSION)
        .author(AUTHOR)
        .about("An Ultrastar song player for the command line written in rust")
        .arg(
            Arg::with_name("songfile")
                .value_name("TXT")
                .help("the song file to play")
                .required(true),
        )
        .get_matches();

    println!("Ultrastar CLI player {} by @man0lis", VERSION);

    // get path from command line arguments, unwrap should not fail because argument is required
    let song_filepath = Path::new(matches.value_of("songfile").unwrap());

    // parse txt file
    let txt_song =
        ultrastar_txt::parse_txt_song(song_filepath).chain_err(|| "could not parse song file")?;
    let header = txt_song.header;
    let lines = txt_song.lines;

    // prepare song
    let bpms = header.bpm / 60.0 / 1000.0;
    let gap = header.gap.unwrap_or(0.0);

    let mut line_iter = lines.into_iter();
    let mut current_line = line_iter.next();
    let mut next_line = line_iter.next();

    // construct path and uri to audio file
    let audio_path = header.audio_path;
    let mut uri = String::from("file://");
    uri.push_str(audio_path.to_str().unwrap());

    // initialize GStreamer
    gst::init().unwrap();

    // create the playbin element
    let playbin = gst::ElementFactory::make("playbin", "playbin")
        .chain_err(|| "failed to create playbin element")?;

    // set the URI to play
    playbin
        .set_property("uri", &uri)
        .chain_err(|| "can't set uri property on playbin")?;

    println!("Playing {} by {}...\n", header.title, header.artist);

    // Start playing
    let ret = playbin.set_state(gst::State::Playing);
    assert_ne!(ret, gst::StateChangeReturn::Failure);

    // connect to the bus
    let bus = playbin.get_bus().unwrap();
    let mut custom_data = CustomData {
        playbin: playbin,
        playing: false,
        terminate: false,
        duration: gst::CLOCK_TIME_NONE,
    };

    // get access to terminal
    //let stdin = stdin();
    let mut stdout = stdout();

    // clear screen
    write!(stdout, "{}", termion::clear::All).chain_err(|| "could not write to stdout")?;

    // begin main loop
    while !custom_data.terminate {
        let msg = bus.timed_pop(10 * gst::MSECOND);

        match msg {
            Some(msg) => {
                handle_message(&mut custom_data, &msg);
            }
            None => {
                if custom_data.playing {
                    let position = custom_data
                        .playbin
                        .query_position(gst::Format::Time)
                        .and_then(|v| v.try_to_time())
                        .unwrap_or(gst::CLOCK_TIME_NONE);

                    // If we didn't know it yet, query the stream duration
                    if custom_data.duration == gst::CLOCK_TIME_NONE {
                        custom_data.duration = custom_data
                            .playbin
                            .query_duration(gst::Format::Time)
                            .and_then(|v| v.try_to_time())
                            .unwrap_or(gst::CLOCK_TIME_NONE);
                    }

                    // calculate current beat
                    let position_ms = position.mseconds().unwrap_or(0) as f32;
                    // don't know why I need the 4.0 but its in the
                    // original game and its not working without it
                    let beat = (position_ms - gap) * (bpms * 4.0);

                    let next_line_start = if next_line.is_some() {
                        next_line.clone().unwrap().start
                    } else {
                        // last line reached, make next if always fail
                        beat as i32 + 100
                    };
                    if beat > next_line_start as f32 {
                        // reprint current line to avoid stale highlights
                        if let &Some(ref line) = &current_line {
                            write!(stdout, "{}", generate_screen(line, beat + 100.0)?)
                                .chain_err(|| "could not write to stdout")?;
                        }

                        if next_line.is_some() {
                            current_line = next_line;
                        };
                        next_line = line_iter.next();
                        // clear screen
                        write!(stdout, "{}", termion::clear::All)
                            .chain_err(|| "could not write to stdout")?;
                    }

                    // print current lyric line
                    if let &Some(ref line) = &current_line {
                        write!(stdout, "{}", generate_screen(line, beat)?)
                            .chain_err(|| "could not write to stdout")?;
                    }
                }
            }
        }
    }
    // end main loop

    // Shutdown pipeline
    let ret = custom_data.playbin.set_state(gst::State::Null);
    assert_ne!(ret, gst::StateChangeReturn::Failure);

    println!("");
    Ok(())
}

fn generate_screen(line: &ultrastar_txt::Line, beat: f32) -> Result<String> {
    let (term_width, _term_height) =
        termion::terminal_size().chain_err(|| "could not get terminal size")?;
    let colored_line = line_to_corlor_str(line, beat);
    let uncolored_line = line_to_str(line);

    let line_vpos = (term_width - uncolored_line.len() as u16) / 2;
    let line_hpos = 50;

    Ok(format!(
        "{}{}",
        termion::cursor::Goto(line_vpos, line_hpos),
        colored_line,
    ))
}

fn line_to_str(line: &ultrastar_txt::Line) -> String {
    let mut line_str = String::new();
    for note in line.notes.iter() {
        match note {
            &ultrastar_txt::Note::Regular {
                start: _,
                duration: _,
                pitch: _,
                ref text,
            } => line_str.push_str(text),
            &ultrastar_txt::Note::Golden {
                start: _,
                duration: _,
                pitch: _,
                ref text,
            } => line_str.push_str(text),
            &ultrastar_txt::Note::Freestyle {
                start: _,
                duration: _,
                pitch: _,
                ref text,
            } => line_str.push_str(text),
            _ => continue,
        };
    }
    line_str
}

#[derive(PartialEq)]
enum NoteType {
    Regular,
    Golden,
    Freestyle,
}

fn line_to_corlor_str(line: &ultrastar_txt::Line, beat: f32) -> String {
    let mut lyric = String::new();
    for note in line.notes.iter() {
        let (start, duration, _pitch, text, note_type) = match note {
            &ultrastar_txt::Note::Regular {
                start,
                duration,
                pitch,
                ref text,
            } => (start, duration, pitch, text, NoteType::Regular),
            &ultrastar_txt::Note::Golden {
                start,
                duration,
                pitch,
                ref text,
            } => (start, duration, pitch, text, NoteType::Golden),
            &ultrastar_txt::Note::Freestyle {
                start,
                duration,
                pitch,
                ref text,
            } => (start, duration, pitch, text, NoteType::Freestyle),
            _ => continue,
        };

        // note is current note or allready played
        if beat >= start as f32 {
            // note is current not -> hightlight it
            if (start + duration) as f32 >= beat {
                if note_type == NoteType::Golden {
                    lyric.push_str(&text.black().on_bright_yellow().to_string());
                } else {
                    lyric.push_str(&text.black().on_bright_white().to_string());
                }
            }
            // note has been played
            else {
                if note_type == NoteType::Golden {
                    lyric.push_str(&text.yellow().to_string());
                } else {
                    lyric.push_str(&text.white().to_string());
                }
            }
        } else {
            if note_type == NoteType::Golden {
                lyric.push_str(&text.bright_yellow().to_string());
            } else {
                lyric.push_str(&text.bright_blue().to_string());
            }
        }
    }
    lyric
}

fn handle_message(custom_data: &mut CustomData, msg: &gst::GstRc<gst::MessageRef>) {
    match msg.view() {
        MessageView::Error(err) => {
            error!(
                "Error received from element {:?}: {} ({:?})",
                msg.get_src().map(|s| s.get_path_string()),
                err.get_error(),
                err.get_debug()
            );
            custom_data.terminate = true;
        }
        MessageView::Eos(..) => {
            info!("End-Of-Stream reached.");
            custom_data.terminate = true;
        }
        MessageView::DurationChanged(_) => {
            // The duration has changed, mark the current one as invalid
            custom_data.duration = gst::CLOCK_TIME_NONE;
        }
        MessageView::StateChanged(state) => if msg.get_src()
            .map(|s| s == custom_data.playbin)
            .unwrap_or(false)
        {
            let new_state = state.get_current();
            let old_state = state.get_old();

            info!(
                "Pipeline state changed from {:?} to {:?}",
                old_state, new_state
            );

            custom_data.playing = new_state == gst::State::Playing;
        },
        _ => (),
    }
}
