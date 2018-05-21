extern crate termion;
extern crate log_update;
extern crate default_editor;
extern crate emoji_commit_type;

use termion::event::Key;
use termion::input::TermRead;
use termion::raw::IntoRawMode;

use std::io::{Write, stderr, stdin};
use std::process::{Command, exit};

use std::env;
use std::fs::File;

use log_update::LogUpdate;
use emoji_commit_type::CommitType;

mod commit_rules;

static PASS: &'static str = "\u{001b}[32m✔\u{001b}[39m";
static FAIL: &'static str = "\u{001b}[31m✖\u{001b}[39m";
static CURSOR: &'static str = "\u{001b}[4m \u{001b}[24m";

fn print_emoji_selector<W: Write>(log_update: &mut LogUpdate<W>, selected: &CommitType) {
    let text = CommitType::iter_variants()
        .map(|t| format!("{}  {}  \u{001b}[90m{:<5}\u{001b}[39m  {}", if t == *selected { "👉" } else { "  " }, t.emoji(), t.bump_level().name().to_lowercase(), t.description()))
        .collect::<Vec<_>>()
        .join("\r\n");

    log_update.render(&text).unwrap();
}

fn select_emoji() -> Option<&'static str> {
    let mut log_update = LogUpdate::new(stderr()).unwrap();
    let mut raw_output = stderr().into_raw_mode().unwrap();

    let mut key_stream = stdin().keys();

    let mut aborted = false;
    let mut selected = CommitType::Breaking;

    loop {
        print_emoji_selector(&mut log_update, &selected);

        match key_stream.next().unwrap().unwrap() {
            Key::Ctrl('c') => { aborted = true; break },
            Key::Char('\n') => break,
            Key::Up | Key::Char('k') | Key::Char('K') => selected = selected.prev_variant().unwrap_or(CommitType::last_variant()),
            Key::Down | Key::Char('j') | Key::Char('J') => selected = selected.next_variant().unwrap_or(CommitType::first_variant()),
            _ => {},
        }
    }

    log_update.clear().unwrap();
    raw_output.flush().unwrap();

    if aborted { None } else { Some(selected.emoji()) }
}

fn collect_commit_message(selected_emoji: &'static str) -> Option<String> {
    let mut log_update = LogUpdate::new(stderr()).unwrap();
    let mut raw_output = stderr().into_raw_mode().unwrap();

    let mut key_stream = stdin().keys();

    let mut aborted = false;
    let mut input = String::new();

    loop {
        let rule_text = commit_rules::CommitRuleIterator::new()
            .map(|t| format!("{} {}", if (t.test)(input.as_str()) { PASS } else { FAIL }, t.text))
            .collect::<Vec<_>>()
            .join("\r\n");
        let text = format!(
            "\r\nRemember the seven rules of a great Git commit message:\r\n\r\n{}\r\n\r\n{}  {}{}",
            rule_text,
            selected_emoji,
            input,
            CURSOR,
        );

        log_update.render(&text).unwrap();

        match key_stream.next().unwrap().unwrap() {
            Key::Ctrl('c') => { aborted = true; break },
            Key::Char('\n') => break,
            Key::Char(c) => input.push(c),
            Key::Backspace => { input.pop(); },
            _ => {},
        }
    }

    log_update.clear().unwrap();
    raw_output.flush().unwrap();

    if aborted { None } else { Some(String::from(input.trim())) }
}

fn abort() -> ! {
    let mut output = stderr();

    write!(output, "Aborted...\n").unwrap();
    output.flush().unwrap();

    exit(1)
}

fn run_cmd(cmd: &mut Command) {
    let status = cmd.status().unwrap();

    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

fn launch_default_editor(out_path: String) {
    let editor = default_editor::get().unwrap();

    run_cmd(Command::new(&editor).arg(out_path))
}

fn launch_git_with_self_as_editor() {
    let self_path = std::env::current_exe().unwrap();

    run_cmd(Command::new("git").arg("commit").env("GIT_EDITOR", self_path))
}

fn collect_information_and_write_to_file(out_path: String) {
    let maybe_emoji = select_emoji();

    if maybe_emoji == None {
        abort();
    }

    if let Some(emoji) = maybe_emoji {
        let maybe_message = collect_commit_message(emoji);

        if maybe_message == None {
            abort();
        }

        if let Some(message) = maybe_message {
            let result = format!("{} {}\n", emoji, message);

            let mut f = File::create(out_path).unwrap();
            f.write_all(result.as_bytes()).unwrap();
        }
    }
}

fn main() {
    if let Some(out_path) = env::args().nth(1) {
        if out_path.ends_with(".git/COMMIT_EDITMSG") {
            collect_information_and_write_to_file(out_path);
        } else {
            launch_default_editor(out_path);
        }
    } else {
        launch_git_with_self_as_editor();
    }
}
