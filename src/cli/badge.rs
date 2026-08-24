use crate::api::schema::{BadgeClearParams, BadgeSetParams, EmptyParams, Method, Request};

/// `herdr badge` — surface the in-binary badge system (socket API
/// `badge.set` / `badge.clear` / `badge.list`) as a CLI verb. Badges are
/// app-global (see `src/badges.rs`); this command is intentionally flat and
/// does not scope to a pane or workspace.
pub(super) fn run_badge_command(args: &[String]) -> std::io::Result<i32> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        print_badge_help();
        return Ok(2);
    };

    match subcommand {
        "set" => badge_set(&args[1..]),
        "clear" => badge_clear(&args[1..]),
        "list" => badge_list(&args[1..]),
        "help" | "--help" | "-h" => {
            print_badge_help();
            Ok(0)
        }
        _ => {
            print_badge_help();
            Ok(2)
        }
    }
}

fn badge_set(args: &[String]) -> std::io::Result<i32> {
    let (key, text, color) = match parse_badge_set_args(args) {
        Ok(params) => params,
        Err(message) => {
            eprintln!("{message}");
            return Ok(2);
        }
    };

    super::print_response(&super::send_request(&Request {
        id: "cli:badge:set".into(),
        method: Method::BadgeSet(BadgeSetParams { key, text, color }),
    })?)
}

fn parse_badge_set_args(args: &[String]) -> Result<(String, String, String), String> {
    const USAGE: &str = "usage: herdr badge set --key KEY --text TEXT --color COLOR";
    // Expand `--key=VALUE` into `--key VALUE` so both forms parse identically.
    let args = super::expand_equals_args(args, &["--key", "--text", "--color"]);

    let mut key: Option<String> = None;
    let mut text: Option<String> = None;
    let mut color: Option<String> = None;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--key" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("missing value for --key".into());
                };
                key = Some(value.clone());
                index += 2;
            }
            "--text" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("missing value for --text".into());
                };
                text = Some(value.clone());
                index += 2;
            }
            "--color" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("missing value for --color".into());
                };
                color = Some(value.clone());
                index += 2;
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown option: {option}"));
            }
            _ => return Err(USAGE.into()),
        }
    }

    let Some(key) = key else {
        return Err(USAGE.into());
    };
    let Some(text) = text else {
        return Err(USAGE.into());
    };
    let Some(color) = color else {
        return Err(USAGE.into());
    };
    Ok((key, text, color))
}

fn badge_clear(args: &[String]) -> std::io::Result<i32> {
    let key = match parse_badge_clear_args(args) {
        Ok(key) => key,
        Err(message) => {
            eprintln!("{message}");
            return Ok(2);
        }
    };

    super::print_response(&super::send_request(&Request {
        id: "cli:badge:clear".into(),
        method: Method::BadgeClear(BadgeClearParams { key }),
    })?)
}

fn parse_badge_clear_args(args: &[String]) -> Result<String, String> {
    const USAGE: &str = "usage: herdr badge clear --key KEY";
    let args = super::expand_equals_args(args, &["--key"]);

    let mut key: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--key" => {
                let Some(value) = args.get(index + 1) else {
                    return Err("missing value for --key".into());
                };
                key = Some(value.clone());
                index += 2;
            }
            option if option.starts_with('-') => {
                return Err(format!("unknown option: {option}"));
            }
            _ => return Err(USAGE.into()),
        }
    }

    let Some(key) = key else {
        return Err(USAGE.into());
    };
    Ok(key)
}

fn badge_list(args: &[String]) -> std::io::Result<i32> {
    if !args.is_empty() {
        eprintln!("usage: herdr badge list");
        return Ok(2);
    }
    super::print_response(&super::send_request(&Request {
        id: "cli:badge:list".into(),
        method: Method::BadgeList(EmptyParams::default()),
    })?)
}

fn print_badge_help() {
    eprintln!("herdr badge commands:");
    eprintln!(
        "  herdr badge set --key KEY --text TEXT --color COLOR   set or replace a badge by key"
    );
    eprintln!("  herdr badge clear --key KEY                          remove a badge by key");
    eprintln!(
        "  herdr badge list                                    list in-memory (IPC-set) badges"
    );
}
