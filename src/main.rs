use chrono::DateTime;
use chrono::offset::Local;
use clap::Parser;
use crossterm::event;
use crossterm::event::{KeyEvent, KeyEventKind};
use crossterm::style::Stylize;
use crossterm::{
    cursor::{Hide, Show},
    execute,
};
use notify_rust::Notification;
use std::io::{Write, stdin, stdout};
use std::ops::Sub;
use std::sync::mpsc;
use std::sync::mpsc::Receiver;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct CliArgs {
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(1..=240), default_value = "50")]
    focus: u8,
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(1..=240), default_value = "10")]
    short_break: u8,
    #[arg(short, long, value_parser = clap::value_parser!(u8).range(1..=240), default_value = "30")]
    long_break: u8,
}

#[derive(Debug, Clone, Eq, PartialEq)]
enum Status {
    Active,
    Paused,
    Resumed,
    Completed,
    Skipped,
    Terminated,
}

fn main() {
    let args = CliArgs::parse();

    let key_event_receiver = prepare_key_event_receiver();

    let mut session = 1;
    let mut status = Status::Active;
    while status != Status::Terminated {
        let session_summary = format!("\u{23F0} Session #{}", session);
        println!(
            "------------------------------\r\n{}\r\n------------------------------",
            session_summary
        );

        // focus
        let focus_secs = input_timer_secs("Focus", args.focus);
        status = run_timer(focus_secs, &key_event_receiver);
        if status == Status::Terminated {
            break;
        }
        if status == Status::Completed {
            notify(session_summary.as_str(), "Time for a break!");
        }

        // break
        let break_secs = if session % 4 == 0 {
            input_timer_secs("Long break", args.long_break)
        } else {
            input_timer_secs("Short break", args.short_break)
        };
        status = run_timer(break_secs, &key_event_receiver);
        if status == Status::Completed {
            notify(session_summary.as_str(), "The break is over!");
        }

        session += 1;
    }
}

fn prepare_key_event_receiver() -> Receiver<KeyEvent> {
    let (tx, rx) = mpsc::channel::<KeyEvent>();
    thread::spawn(move || {
        loop {
            if let event::Event::Key(key) = event::read().unwrap()
                && key.kind == KeyEventKind::Press
            {
                tx.send(key).unwrap();
            }
        }
    });
    rx
}

fn input_timer_secs(desc: &str, default_mins: u8) -> u64 {
    print!("{} (default {}): ", desc, default_mins);
    stdout().flush().unwrap();

    let minutes = read_minutes();
    (if minutes == 0 { default_mins } else { minutes }) as u64 * 60
}

fn read_minutes() -> u8 {
    let mut minutes = String::new();
    stdin()
        .read_line(&mut minutes)
        .expect("Could not read std input");
    if !minutes.trim().is_empty() {
        return minutes.trim().parse::<u8>().unwrap();
    }
    0
}

fn run_timer(seconds: u64, rx: &Receiver<KeyEvent>) -> Status {
    enable_raw_terminal();

    let mut status = Status::Active;
    let mut accumulated_pause = 0;
    let mut pause_start = Instant::now();
    let start = Instant::now();
    let start_time = Local::now();
    loop {
        let remaining = calc_remaining(
            seconds,
            start,
            pause_start,
            accumulated_pause,
            status.clone(),
        );
        if remaining <= 0 {
            status = Status::Completed;
            break;
        }

        print_timer(status.clone(), remaining);

        let recv_result = rx.try_recv();
        match recv_result {
            Ok(key) => {
                status = update_status(key, status);
                if status == Status::Skipped || status == Status::Terminated {
                    break;
                }
                if status == Status::Paused {
                    pause_start = Instant::now();
                    continue;
                }
                if status == Status::Resumed {
                    accumulated_pause += pause_start.elapsed().as_secs();
                    status = Status::Active;
                    continue;
                }
            }
            Err(_) => {
                //println!("{}", e);
            }
        }

        thread::sleep(Duration::from_secs(1));
    }

    print_interval(status.clone(), start_time, Local::now(), accumulated_pause);

    disable_raw_terminal();

    status
}

fn calc_remaining(
    seconds: u64,
    start: Instant,
    pause_start: Instant,
    accumulated_pause: u64,
    status: Status,
) -> i64 {
    let elapsed = start.elapsed().as_secs();
    let remaining = if status == Status::Paused {
        seconds + accumulated_pause + pause_start.elapsed().as_secs() - elapsed
    } else {
        seconds + accumulated_pause - elapsed
    } as i64;
    remaining
}

fn enable_raw_terminal() {
    crossterm::terminal::enable_raw_mode().unwrap();
    let mut stdout = stdout();
    execute!(stdout, Hide).unwrap();
}

fn disable_raw_terminal() {
    crossterm::terminal::disable_raw_mode().unwrap();
    let mut stdout = stdout();
    execute!(stdout, Show).unwrap();
}

fn print_timer(status: Status, remaining: i64) {
    if status == Status::Paused {
        print!("\r{}", format_seconds(remaining).red());
    } else {
        print!("\r{}", format_seconds(remaining));
    }
    stdout().flush().unwrap();
}

fn format_seconds(seconds: i64) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

fn print_interval(
    status: Status,
    start_time: DateTime<Local>,
    end_time: DateTime<Local>,
    accumulated_pause: u64,
) {
    if status == Status::Skipped || (status == Status::Completed && accumulated_pause > 0) {
        print!(
            "\r{}-{} ({})\r\n",
            format_time(start_time).yellow(),
            format_time(end_time).yellow(),
            (end_time.sub(start_time).as_seconds_f64() as u64 / 60)
                .to_string()
                .yellow()
        );
    } else if status == Status::Completed {
        print!(
            "\r{}-{}\r\n",
            format_time(start_time).green(),
            format_time(end_time).green()
        );
    }
    stdout().flush().unwrap();
}

fn format_time(date_time: DateTime<Local>) -> String {
    date_time.format("%H:%M").to_string()
}

fn update_status(key: KeyEvent, current_status: Status) -> Status {
    if key.code == event::KeyCode::Esc
        || (key.code == event::KeyCode::Char('c') && key.modifiers == event::KeyModifiers::CONTROL)
    {
        return Status::Terminated;
    } else if key.code == event::KeyCode::Tab {
        return Status::Skipped;
    } else if key.code == event::KeyCode::Char(' ') {
        return if current_status == Status::Active {
            Status::Paused
        } else {
            Status::Resumed
        };
    }
    current_status
}

fn notify(summary: &str, body: &str) {
    Notification::new()
        .summary(summary)
        .body(body)
        .appname("Pomodoro")
        .timeout(0) // never expires
        .show()
        .expect("Could not show notification");
}
