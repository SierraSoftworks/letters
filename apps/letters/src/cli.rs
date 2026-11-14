//! The command line: `letters [FILE]`, the launch-time measurements, and the `export` and `import` subcommands.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
Usage: letters [OPTIONS] [FILE]
       letters export --to md|html|docx|pdf [OPTIONS] FILE
       letters import [OPTIONS] PATH...

Opens FILE, or a new document when none is given.

Options:
      --launch-report      Print how long each stage of launch took, then exit after the first frame
      --baseline           The same for an empty window, which is what launches are measured against
      --bench-launch[=N]   Launch N times (default 20) and check what Letters adds to an empty window
      --typing-report[=N]  Type N characters (default 200) and check the keystroke budget, saving nothing
      --headless           With the four above: paint into memory, without a window
  -V, --version
  -h, --help

Export writes FILE as Markdown, one HTML file, a Word document or a PDF, beside it unless told otherwise:
      --to FORMAT          The format to write: md, html, docx or pdf
  -o, --output PATH        Where to write, or - for standard output
  -f, --force              Replace the output if it is already there

Import makes a document beside each Markdown or Word file, leaves the original alone and prints one line
per file:
  -r, --recursive          Look through the folders inside each PATH as well
      --json               Print each line as a JSON object

Environment:
  LETTERS_RENDERER=cpu     Never use the GPU
  LETTERS_RENDERER=gpu     Use the GPU even when it is a software one
";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Run(Options),
    Export(Export),
    Import(Import),
    Version,
    Help,
}

/// `letters export`: one document written in another format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Export {
    pub file: PathBuf,
    pub to: Format,
    /// Where to write. `None` is beside the document, under its name.
    pub output: Option<Output>,
    /// Whether to replace an output that is already there.
    pub force: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Html,
    Docx,
    Pdf,
}

impl Format {
    /// The extension of a file in this format.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Docx => "docx",
            Self::Pdf => "pdf",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    File(PathBuf),
    Stdout,
}

/// `letters import`: a document made from each Markdown or Word file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Import {
    /// Files, and folders whose Markdown and Word files are meant.
    pub paths: Vec<PathBuf>,
    pub recursive: bool,
    /// Whether each line of the report is a JSON object.
    pub json: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub file: Option<PathBuf>,
    pub mode: Mode,
    /// Paints the first frame into memory. Only the measuring modes take it.
    pub headless: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Edit,
    LaunchReport,
    /// An empty window in place of the app. See [`crate::baseline`].
    Baseline,
    BenchLaunch {
        runs: u32,
    },
    TypingReport {
        keys: u32,
    },
}

/// Parses the arguments after the program name.
///
/// `export` and `import` are subcommands only as the first argument, so a file of either name still opens
/// after `--` or as `./export`.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut args = args.into_iter().peekable();
    match args.peek().and_then(|first| first.to_str()) {
        Some("export") => return parse_export(args.skip(1)),
        Some("import") => return parse_import(args.skip(1)),
        _ => {}
    }
    let mut options = Options::default();
    let mut only_files = false;
    for arg in args {
        let flag = arg.to_str().filter(|text| !only_files && text.starts_with('-') && text.len() > 1);
        match flag {
            Some("--") => only_files = true,
            Some("-V" | "--version") => return Ok(Command::Version),
            Some("-h" | "--help") => return Ok(Command::Help),
            Some("--launch-report") => options.mode = Mode::LaunchReport,
            Some("--baseline") => options.mode = Mode::Baseline,
            Some("--bench-launch") => options.mode = Mode::BenchLaunch { runs: 20 },
            Some("--typing-report") => options.mode = Mode::TypingReport { keys: 200 },
            Some("--headless") => options.headless = true,
            Some(flag) => match flag.split_once('=').map(|(name, count)| (name, count.parse())) {
                Some(("--bench-launch", Ok(runs))) if runs > 0 => options.mode = Mode::BenchLaunch { runs },
                Some(("--typing-report", Ok(keys))) if keys > 0 => options.mode = Mode::TypingReport { keys },
                Some(("--bench-launch" | "--typing-report", _)) => {
                    return Err(format!("{flag}: expected a count above zero"));
                }
                _ => return Err(format!("unknown option {flag}")),
            },
            None if options.file.is_some() => return Err("only one file can be opened at a time".into()),
            None => options.file = Some(arg.into()),
        }
    }
    if options.headless && options.mode == Mode::Edit {
        return Err("--headless needs --launch-report, --baseline, --bench-launch or --typing-report".into());
    }
    Ok(Command::Run(options))
}

/// One argument of a subcommand: a flag with the value written into it after `=`, or a path.
enum Arg<'a> {
    Flag(&'a str, Option<&'a str>),
    Path,
}

fn classify(arg: &OsString, only_paths: bool) -> Arg<'_> {
    match arg.to_str().filter(|text| !only_paths && text.starts_with('-') && text.len() > 1) {
        Some(flag) if flag.starts_with("--") => match flag.split_once('=') {
            Some((name, value)) => Arg::Flag(name, Some(value)),
            None => Arg::Flag(flag, None),
        },
        Some(flag) => Arg::Flag(flag, None),
        None => Arg::Path,
    }
}

fn parse_export(args: impl Iterator<Item = OsString>) -> Result<Command, String> {
    let (mut file, mut to, mut output, mut force) = (None, None, None, false);
    let mut only_paths = false;
    let mut args = args;
    while let Some(arg) = args.next() {
        // A flag's value is written into it after `=`, or is the argument after it.
        let mut value = |inline: Option<&str>, name: &str| match inline {
            Some(value) => Ok(OsString::from(value)),
            None => args.next().ok_or(format!("{name} needs a value")),
        };
        match classify(&arg, only_paths) {
            Arg::Flag("--", None) => only_paths = true,
            Arg::Flag("-V" | "--version", None) => return Ok(Command::Version),
            Arg::Flag("-h" | "--help", None) => return Ok(Command::Help),
            Arg::Flag("-f" | "--force", None) => force = true,
            Arg::Flag(name @ "--to", inline) => {
                to = Some(match value(inline, name)?.to_str() {
                    Some("md" | "markdown") => Format::Markdown,
                    Some("html") => Format::Html,
                    Some("docx") => Format::Docx,
                    Some("pdf") => Format::Pdf,
                    _ => return Err("--to takes md, html, docx or pdf".into()),
                });
            }
            Arg::Flag(name @ ("-o" | "--output"), inline) => {
                let path = value(inline, name)?;
                output = Some(if path == "-" { Output::Stdout } else { Output::File(path.into()) });
            }
            Arg::Flag(name, _) => return Err(format!("export: unknown option {name}")),
            Arg::Path if file.is_some() => return Err("export takes one file at a time".into()),
            Arg::Path => file = Some(PathBuf::from(arg)),
        }
    }
    let file = file.ok_or("export needs the file to export")?;
    let to = to.ok_or("export needs --to md, --to html or --to docx")?;
    Ok(Command::Export(Export { file, to, output, force }))
}

fn parse_import(args: impl Iterator<Item = OsString>) -> Result<Command, String> {
    let mut import = Import::default();
    let mut only_paths = false;
    for arg in args {
        match classify(&arg, only_paths) {
            Arg::Flag("--", None) => only_paths = true,
            Arg::Flag("-V" | "--version", None) => return Ok(Command::Version),
            Arg::Flag("-h" | "--help", None) => return Ok(Command::Help),
            Arg::Flag("-r" | "--recursive", None) => import.recursive = true,
            Arg::Flag("--json", None) => import.json = true,
            Arg::Flag(name, _) => return Err(format!("import: unknown option {name}")),
            Arg::Path => import.paths.push(arg.into()),
        }
    }
    if import.paths.is_empty() {
        return Err("import needs a file or a folder to import".into());
    }
    Ok(Command::Import(import))
}
