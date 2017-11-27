extern crate chardet;
extern crate colored;
extern crate encoding;
extern crate env_logger;
extern crate gstreamer as gst;
#[macro_use]
extern crate log;
extern crate ultrastar_txt;

use std::env;
use std::fs::File;
use std::io;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use gst::MessageView;
use gst::prelude::*;
use colored::*;
use ultrastar_txt::NoteType;

const VERSION: &'static str = env!("CARGO_PKG_VERSION");

struct CustomData {
    playbin: gst::Element,    // Our one and only element
    playing: bool,            // Are we in the PLAYING state?
    terminate: bool,          // Should we terminate execution?
    duration: gst::ClockTime, // How long does this media last, in nanoseconds
}

fn main() {
    let _ = env_logger::init();

    println!("Ultrastar CLI player {} by manolis", VERSION);

    // hanlde cmd line args
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        println!("Usage: {} <UltraStar Song TXT>", &args[0]);
        std::process::exit(1);
    }
    let song_filepath = Path::new(&args[1]);

    // open txt file
    let mut file = File::open(song_filepath).expect("Could not open file :(");
    let mut reader: Vec<u8> = Vec::new();

    // read file, detect encoding and decode to String
    file.read_to_end(&mut reader)
        .expect("Could not read file :(");
    let chardet_result = chardet::detect(&reader);
    let whatwg_label = chardet::charset2encoding(&chardet_result.0);
    let coder = encoding::label::encoding_from_whatwg_label(whatwg_label);
    let txt = coder
        .unwrap()
        .decode(&reader, encoding::DecoderTrap::Ignore)
        .expect("Could not decode file :(");

    // parse txt file
    let header =
        ultrastar_txt::parse_txt_header_str(txt.as_ref()).expect("Could not parse header :(");
    let lines =
        ultrastar_txt::parse_txt_lines_str(txt.as_ref()).expect("Could not parse lyric lines :(");

    // prepare song
    let bpms = header.bpm / 60.0 / 1000.0;
    let gap = header.gap.unwrap_or(0.0);

    let mut line_iter = lines.into_iter();
    let mut current_line = line_iter.next();
    let mut next_line = line_iter.next();

    // construct path and uri to audio file
    let mut audio_path = PathBuf::from(song_filepath.parent().unwrap());
    audio_path.push(header.audio_path);
    let full_audio_path = audio_path.canonicalize().unwrap();

    let mut uri = String::from("file://");
    uri.push_str(full_audio_path.to_str().unwrap());

    // initialize GStreamer
    gst::init().unwrap();

    // create the playbin element
    let playbin = gst::ElementFactory::make("playbin", "playbin")
        .expect("Failed to create playbin element :(");

    // set the URI to play
    playbin
        .set_property("uri", &uri)
        .expect("Can't set uri property on playbin :(");

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
                        if next_line.is_some() {
                            current_line = next_line;
                        };
                        next_line = line_iter.next();
                        println!("");
                    }

                    // print current lyric line
                    let mut lyric = String::new();
                    if let &Some(ref line) = &current_line {
                        for note in line.notes.iter() {
                            // note is current note or allready played
                            if beat >= note.start as f32 {
                                // note is current not -> hightlight it
                                if (note.start + note.duration) as f32 >= beat {
                                    if note.notetype == NoteType::Golden {
                                        lyric.push_str(
                                            &note.text.black().on_bright_yellow().to_string(),
                                        );
                                    } else {
                                        lyric.push_str(
                                            &note.text.black().on_bright_white().to_string(),
                                        );
                                    }
                                }
                                // note has been played
                                else {
                                    if note.notetype == NoteType::Golden {
                                        lyric.push_str(&note.text.yellow().to_string());
                                    } else {
                                        lyric.push_str(&note.text.white().to_string());
                                    }
                                }
                            } else {
                                if note.notetype == NoteType::Golden {
                                    lyric.push_str(&note.text.bright_yellow().to_string());
                                } else {
                                    lyric.push_str(&note.text.bright_blue().to_string());
                                }
                            }
                        }
                    }

                    print!("\r{}", lyric);
                    io::stdout().flush().unwrap();
                }
            }
        }
    }
    // end main loop

    // Shutdown pipeline
    let ret = custom_data.playbin.set_state(gst::State::Null);
    assert_ne!(ret, gst::StateChangeReturn::Failure);
    
    println!("");
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
                old_state,
                new_state
            );

            custom_data.playing = new_state == gst::State::Playing;
        },
        _ => (),
    }
}
