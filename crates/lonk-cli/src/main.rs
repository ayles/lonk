use lonk::{Program, Syntax};
use std::ffi::OsString;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const HELP: &str = "Lonk programming language

Usage:
  lonk eval FILE [PATH]     Evaluate the result or a selected value
  lonk check FILE [PATH]    Check the whole source and print its selected type
  lonk fmt FILE [--check]   Format source to stdout, or check its formatting
  lonk ast FILE            Print the syntax tree for debugging
  lonk --help              Show this help
  lonk --version           Show the version

PATH uses dot-separated fields and decimal array indices, such as packages.0.name.
Checking never evaluates values. Formatting only requires valid syntax and never
changes the input file. With --check, fmt produces no output on success and exits
with status 1 if formatting differs. Use -- before filenames starting with '-'.
The short options -h and -V mean --help and --version.
";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Command {
    Eval,
    Check,
    Fmt,
    Ast,
}

enum Action {
    Help,
    Version,
    Run {
        command: Command,
        file: PathBuf,
        path: Vec<String>,
        check_format: bool,
    },
}

fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Action, String> {
    let mut args = args.into_iter();
    let first = args.next().ok_or("a command is required")?;
    let rest: Vec<_> = args.collect();
    if first == "--help" || first == "-h" || first == "--version" || first == "-V" {
        if !rest.is_empty() {
            return Err(format!(
                "unexpected argument after {}",
                first.to_string_lossy()
            ));
        }
        return Ok(if first == "--help" || first == "-h" {
            Action::Help
        } else {
            Action::Version
        });
    }
    let command = match first.to_str() {
        Some("eval") => Command::Eval,
        Some("check") => Command::Check,
        Some("fmt") => Command::Fmt,
        Some("ast") => Command::Ast,
        _ => return Err(format!("unknown command '{}'", first.to_string_lossy())),
    };
    if rest.len() == 1 && (rest[0] == "--help" || rest[0] == "-h") {
        return Ok(Action::Help);
    }

    let mut positional = Vec::new();
    let mut check_format = false;
    let mut options = true;
    for arg in rest {
        if options && arg == "--" {
            options = false;
        } else if options && arg == "--check" && command == Command::Fmt {
            if check_format {
                return Err("--check was specified more than once".into());
            }
            check_format = true;
        } else if options && arg.to_string_lossy().starts_with('-') {
            return Err(format!("unknown option '{}'", arg.to_string_lossy()));
        } else {
            positional.push(arg);
        }
    }
    let limit = if matches!(command, Command::Eval | Command::Check) {
        2
    } else {
        1
    };
    if positional.len() > limit {
        return Err(format!(
            "unexpected argument '{}'",
            positional[limit].to_string_lossy()
        ));
    }
    let mut positional = positional.into_iter();
    let file = PathBuf::from(positional.next().ok_or("FILE is required")?);
    let path = match positional.next() {
        Some(path) => {
            let path = path.into_string().map_err(|_| "PATH must be valid UTF-8")?;
            if path.split('.').any(str::is_empty) {
                return Err("PATH must contain nonempty dot-separated segments".into());
            }
            path.split('.').map(str::to_owned).collect()
        }
        None => Vec::new(),
    };
    Ok(Action::Run {
        command,
        file,
        path,
        check_format,
    })
}

fn execute(action: Action) -> Result<String, String> {
    let (command, file, path, check_format) = match action {
        Action::Help => return Ok(HELP.into()),
        Action::Version => return Ok(format!("lonk {}\n", env!("CARGO_PKG_VERSION"))),
        Action::Run {
            command,
            file,
            path,
            check_format,
        } => (command, file, path, check_format),
    };
    let source =
        std::fs::read_to_string(&file).map_err(|error| format!("{}: {error}", file.display()))?;
    if matches!(command, Command::Fmt | Command::Ast) {
        let syntax = Syntax::parse(&source)
            .map_err(|error| format!("{}:{}", file.display(), error.render(&source)))?;
        if command == Command::Ast {
            return Ok(format!("{syntax:#?}\n"));
        }
        let formatted = syntax.format();
        return if !check_format {
            Ok(formatted)
        } else if source == formatted {
            Ok(String::new())
        } else {
            Err(format!("{}: formatting differs", file.display()))
        };
    }

    let program = Program::prepare(&source)
        .map_err(|error| format!("{}:{}", file.display(), error.render(&source)))?;
    let path: Vec<_> = path.iter().map(String::as_str).collect();
    if command == Command::Check {
        return program
            .type_at(&path)
            .map(|ty| format!("{ty}\n"))
            .map_err(|error| format!("{}:{}", file.display(), error.render(&source)));
    }
    program
        .evaluate(&path)
        .map(|value| format!("{value}\n"))
        .map_err(|error| {
            let rendered = program.render_diagnostic(&error);
            if rendered.starts_with("<stdlib>:") {
                rendered
            } else {
                format!("{}:{rendered}", file.display())
            }
        })
}

fn main() -> ExitCode {
    let action = match parse(std::env::args_os().skip(1)) {
        Ok(action) => action,
        Err(error) => {
            eprintln!("error: {error}\nRun 'lonk --help' for usage.");
            return ExitCode::from(2);
        }
    };
    match execute(action) {
        Ok(output) => match io::stdout().lock().write_all(output.as_bytes()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) if error.kind() == io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("error: writing stdout: {error}");
                ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}
