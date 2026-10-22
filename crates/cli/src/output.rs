//! Text output for people. Agents and scripts use `--json`.

use std::io::IsTerminal;

use tuma_core::runner::{Outcome, Summary};

struct Style {
    color: bool,
}

impl Style {
    fn new() -> Self {
        Self {
            color: std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    fn green(&self, text: &str) -> String {
        self.paint("32", text)
    }

    fn red(&self, text: &str) -> String {
        self.paint("31", text)
    }

    fn yellow(&self, text: &str) -> String {
        self.paint("33", text)
    }

    fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }
}

/// Prints one result. `full` adds headers and the body, for `tuma send`.
pub fn outcome(o: &Outcome, full: bool) {
    let s = Style::new();
    let title = o.path.as_deref().unwrap_or(&o.url);
    match (&o.error, o.status) {
        (Some(error), _) => {
            println!("{} {} {title}", s.red("✗"), o.method);
            println!("  {}", s.red(error));
        }
        (None, Some(status)) => {
            let mark = if o.passed {
                s.green("✓")
            } else {
                s.red("✗")
            };
            let status_text = format!("{status} {}", o.reason);
            let status_text = if status < 400 {
                s.green(&status_text)
            } else {
                s.red(&status_text)
            };
            let time = o.time_ms.map(|t| format!("{t} ms")).unwrap_or_default();
            let size = o.size.map(format_size).unwrap_or_default();
            println!(
                "{mark} {} {title}  {status_text}  {}",
                o.method,
                s.dim(&format!("{time} · {size}"))
            );
            // A saved request's title is its path, so show the URL too.
            if full && o.path.is_some() {
                println!("  {}", s.dim(&o.url));
            }
        }
        (None, None) => {}
    }
    if full {
        for header in &o.headers {
            println!("  {}", s.dim(&format!("{}: {}", header.name, header.value)));
        }
        if let Some(body) = &o.body {
            println!();
            println!("{body}");
            if o.body_truncated {
                println!(
                    "{}",
                    s.dim(&format!(
                        "… cut short; the body is {}",
                        o.size.map(format_size).unwrap_or_default()
                    ))
                );
            }
        }
        if !o.checks.is_empty() {
            println!();
        }
    }
    for check in &o.checks {
        if check.passed {
            if full {
                println!("  {} {}", s.green("✓"), check.check);
            }
        } else {
            println!(
                "  {} {} {}",
                s.red("✗"),
                check.check,
                s.dim(&format!("(got {})", check.actual))
            );
        }
    }
    if !o.unresolved.is_empty() {
        let names: Vec<String> = o
            .unresolved
            .iter()
            .map(|n| format!("{{{{{n}}}}}"))
            .collect();
        println!("  {} no value for {}", s.yellow("!"), names.join(", "));
    }
}

pub fn summary(summary: &Summary) {
    let s = Style::new();
    let requests = format!(
        "{} request{}",
        summary.requests,
        if summary.requests == 1 { "" } else { "s" }
    );
    let line = if summary.failed == 0 {
        s.green(&format!("{requests} passed, {} checks", summary.checks))
    } else {
        s.red(&format!(
            "{} of {requests} failed ({} of {} checks failed)",
            summary.failed, summary.failed_checks, summary.checks
        ))
    };
    println!("\n{line}");
}

fn format_size(bytes: usize) -> String {
    match bytes {
        b if b < 1024 => format!("{b} B"),
        b if b < 1024 * 1024 => format!("{:.1} KB", b as f64 / 1024.0),
        b => format!("{:.1} MB", b as f64 / 1024.0 / 1024.0),
    }
}
