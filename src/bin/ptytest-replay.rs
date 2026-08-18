use ptytest::{ReplayOptions, replay_failure_bundle_with_options};
use std::path::PathBuf;

fn main() {
    let mut arguments = std::env::args_os().skip(1);
    let Some(path) = arguments.next() else {
        usage();
    };
    let mut options = ReplayOptions::default();
    let mut dump_events = false;
    let mut step = false;
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--dump-events") => dump_events = true,
            Some("--step") => {
                step = true;
                options = options.with_checkpoints();
            }
            Some("--event") => {
                let Some(value) = arguments.next().and_then(|value| value.into_string().ok())
                else {
                    usage();
                };
                let Ok(sequence) = value.parse() else {
                    usage();
                };
                options = options.at_event(sequence);
            }
            Some("--byte") => {
                let Some(value) = arguments.next().and_then(|value| value.into_string().ok())
                else {
                    usage();
                };
                let Ok(offset) = value.parse() else {
                    usage();
                };
                options = options.at_output_byte(offset);
            }
            _ => usage(),
        }
    }
    match replay_failure_bundle_with_options(PathBuf::from(path), options) {
        Ok(result) => {
            if dump_events {
                for event in result.events() {
                    println!(
                        "#{:04} +{}ns {} {}",
                        event.sequence(),
                        event.elapsed_ns(),
                        event.kind(),
                        event.payload()
                    );
                }
            }
            if step {
                for checkpoint in result.checkpoints() {
                    println!(
                        "== after #{} {} ==",
                        checkpoint.event_sequence(),
                        checkpoint.kind()
                    );
                    print!("{}", checkpoint.screen());
                }
            }
            println!("replayed {} read/resize events", result.events_replayed());
            print!("{}", result.screen());
        }
        Err(error) => {
            eprintln!("ptytest-replay: {error}");
            std::process::exit(1);
        }
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: ptytest-replay <failure-bundle> [--dump-events] [--step] [--event <sequence> | --byte <output-offset>]"
    );
    std::process::exit(2)
}
