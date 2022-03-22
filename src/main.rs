use std::env;
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{Write, stderr};
use std::process::{Command, exit};
use std::path::PathBuf;
use std::str::FromStr;
use std::ffi::CString;
use libc::system;

use crossterm::event::{Event, KeyEvent, KeyCode, KeyModifiers};
use crossterm::{terminal, event};

use default_editor;
use emoji_commit_type::CommitType;
use log_update::LogUpdate;
use structopt::StructOpt;
#[cfg(target_os = "windows")]
use ansi_term::enable_ansi_support;
use ansi_term::Colour::{RGB, Green, Red, White};
use path_slash::PathExt;

mod commit_rules;
mod git;

impl fmt::Display for commit_rules::CommitRuleValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", if self.pass {
            Green.paint("✔")
        } else {
            Red.paint("✖")
        }, self.description)
    }
}

fn print_emoji_selector<W: Write>(log_update: &mut LogUpdate<W>, selected: &CommitType) {
    let text = CommitType::iter_variants()
        .map(|t| format!("{}  {}  \u{001b}[90m{:<5}\u{001b}[39m  {}", if t == *selected { "👉" } else { "  " }, t.emoji(), t.bump_level().name().to_lowercase(), t.description()))
        .collect::<Vec<_>>()
        .join("\r\n");

    log_update.render(&text).unwrap();
}

fn commit_type_at_index (index: u8) -> Option<CommitType> {
    return CommitType::iter_variants().nth(index as usize);
}

fn select_emoji() -> Option<&'static str> {
    let mut log_update = LogUpdate::new(stderr()).unwrap();
    terminal::enable_raw_mode().unwrap();

    let mut aborted = false;
    let mut selected = CommitType::Breaking;

    // Clear possibly printed "hint" from git
    log_update.render("").unwrap();

    loop {
        print_emoji_selector(&mut log_update, &selected);

        let event = if let Event::Key(event) = event::read().ok()? { event } else { continue };

        match event {
            KeyEvent {code: KeyCode::Char('c' | 'C'), modifiers: KeyModifiers::CONTROL} => { aborted = true; break },
            KeyEvent {code: KeyCode::Enter, modifiers: _} => break,
            KeyEvent {code: KeyCode::Char('k' | 'K') | KeyCode::Up, modifiers: _} => selected = selected.prev_variant().unwrap_or(CommitType::last_variant()),
            KeyEvent {code: KeyCode::Char('j' | 'J') | KeyCode::Down, modifiers: _} => selected = selected.next_variant().unwrap_or(CommitType::first_variant()),
            KeyEvent {code: KeyCode::Char(key @ '1' ..= '9'), modifiers: _} => { commit_type_at_index((key as u8) - ('1' as u8)).map(|t| selected = t); },
            _ => {},
        }
    }

    log_update.clear().unwrap();
    terminal::disable_raw_mode().unwrap();

    if aborted { None } else { Some(selected.emoji()) }
}

fn collect_commit_message(selected_emoji: &'static str, launch_editor: &mut bool) -> Option<String> {
    let mut log_update = LogUpdate::new(stderr()).unwrap();
    terminal::enable_raw_mode().unwrap();

    let mut aborted = false;
    let mut input = String::new();

    loop {
        let rule_text = commit_rules::check_message(&input)
            .map(|result| format!("{}", result))
            .collect::<Vec<_>>()
            .join("\r\n");
        let text = format!(
            "\r\nRemember the seven rules of a great Git commit message:\r\n\r\n{}\r\n\r\n{}\r\n{}  {}{}",
            rule_text,
            RGB(105, 105, 105).paint("Enter - finish, Ctrl-C - abort, Ctrl-E - continue editing in $EDITOR"),
            selected_emoji,
            input,
            White.underline().paint(" ")
        );

        log_update.render(&text).unwrap();

        let event = if let Event::Key(event) = event::read().ok()? { event } else { continue };

        match event {
            KeyEvent {code: KeyCode::Char('c' | 'C'), modifiers: KeyModifiers::CONTROL} => { aborted = true; break },
            KeyEvent {code: KeyCode::Char('e' | 'E'), modifiers: KeyModifiers::CONTROL} => { *launch_editor = true; break },
            KeyEvent {code: KeyCode::Enter, modifiers: _} => break,
            KeyEvent {code: KeyCode::Backspace, modifiers: _} => { input.pop(); },
            KeyEvent {code: KeyCode::Char(c), modifiers: _} => input.push(c),
            _ => {},
        }
    }

    log_update.clear().unwrap();
    terminal::disable_raw_mode().unwrap();

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

fn launch_default_editor(out_path: PathBuf) {
    let editor = default_editor::get().unwrap();
    let cmd = CString::new(format!("{} {}", editor, out_path.display())).unwrap();

    unsafe {
        system(cmd.as_ptr());
    }
}

fn launch_git_with_self_as_editor() {
    let self_path = std::env::current_exe().unwrap().to_slash().unwrap();

    run_cmd(Command::new("git").arg("commit").env("GIT_EDITOR", self_path))
}

fn collect_information_and_write_to_file(out_path: PathBuf) {
    let maybe_emoji = select_emoji();

    if maybe_emoji == None {
        abort();
    }

    if let Some(emoji) = maybe_emoji {
        let mut launch_editor = false;
        let maybe_message = collect_commit_message(emoji, &mut launch_editor);

        if maybe_message == None {
            abort();
        }

        if let Some(message) = maybe_message {
            let result = format!("{} {}\n", emoji, message);

            let mut f = File::create(out_path.clone()).unwrap();
            f.write_all(result.as_bytes()).unwrap();
            drop(f);

            if launch_editor {
                launch_default_editor(out_path);
            }
        }
    }
}

#[derive(Debug)]
struct ValidationError;

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", "One or more commits has validation errors")
    }
}

impl Error for ValidationError {}

fn validate(refspecs: Vec<String>) -> Result<(), Box<dyn Error>> {
    let repo_path = env::current_dir()?;
    let messages = git::get_commit_messages(repo_path, refspecs)?;

    let mut has_errors = false;
    for message in messages {
        let validation_errors = commit_rules::check_message_with_emoji(&message)
            .filter_map(|result| {
                if !result.pass { Some(format!("\t{}", result)) } else { None }
            })
            .collect::<Vec<_>>();
        if validation_errors.len() > 0 {
            has_errors = true;
            let validation_result_text = validation_errors.join("\r\n");
            let text = format!(
                "\r\nCommit:\r\n\t{}\r\nValidation Errors:\r\n{}\r\n",
                message,
                validation_result_text,
            );
            println!("{}", text);
        }
    }
    if has_errors {Err(Box::new(ValidationError{}))} else { Ok(()) }
}

#[derive(Debug)]
enum OutPath {
    EditMessage(PathBuf),
    RebaseTodo(PathBuf),
    AddPHunkEdit(PathBuf),
}

impl FromStr for OutPath {
    type Err = String;
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let path = PathBuf::from(raw);
        if path.ends_with(".git/COMMIT_EDITMSG") {
            Ok(OutPath::EditMessage(path))
        } else if path.ends_with(".git/rebase-merge/git-rebase-todo") {
            Ok(OutPath::RebaseTodo(path))
        } else if path.ends_with(".git/addp-hunk-edit.diff") {
            Ok(OutPath::AddPHunkEdit(path))
        } else {
            Err(String::from(format!("Must end with one of the following: \r\n\t{}\r\n\t{}\r\n\t{}\r\nGot the following path: {:?}", ".git/COMMIT_EDITMSG", ".git/rebase-merge/git-rebase-todo", ".git/addp-hunk-edit.diff", path)))
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(about = "Make your git logs beautiful and readable with the help of emojis 🎉")]
struct Opt {
    #[structopt(
        help = "Path passed by Git where commit message should be written to. Path will be passed through to $EDITOR if doing a interactive rebase.",
        group = "mutually-exclusive",
        hidden = true,
    )]
    out_path: Option<OutPath>,

    #[structopt(
        long = "validate",
        help = "Validate the provided refspecs against the commit rules. Refspecs can be passed as is or in the special notation defined in git-rev-list.",
        group = "mutually-exclusive",
    )]
    refspecs: Option<Vec<String>>,
}

fn main() -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    {
        enable_ansi_support().unwrap();
    }
    
    match Opt::from_args() {
        Opt {out_path: None, refspecs: Some(refspecs)} => validate(refspecs),
        Opt {out_path: Some(OutPath::EditMessage(out_path)), refspecs: None} => {
            collect_information_and_write_to_file(out_path);
            Ok(())
        },
        Opt {
            out_path: Some(OutPath::RebaseTodo(out_path) | OutPath::AddPHunkEdit(out_path)),
            refspecs: None,
        } => {
            launch_default_editor(out_path);
            Ok(())
        },
        _ => {
            launch_git_with_self_as_editor();
            Ok(())
        }
    }
}
