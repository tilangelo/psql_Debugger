//! Общая остановка своего PostgreSQL для Linux и Windows.
use std::{
    io,
    path::Path,
    process::{Command, Output},
};

fn is_running(pg_ctl: &Path, data: &Path) -> io::Result<bool> {
    let output = Command::new(pg_ctl)
        .arg("-D")
        .arg(data)
        .arg("status")
        .output()?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(3) => Ok(false), // pg_ctl: сервер не работает.
        _ => Err(failure("pg_ctl status", &output)),
    }
}

fn failure(label: &str, output: &Output) -> io::Error {
    io::Error::other(format!(
        "{label}: {}\n{}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

pub fn stop(pg_ctl: &Path, data: &Path) -> io::Result<()> {
    if !is_running(pg_ctl, data)? {
        return Ok(());
    }
    let output = Command::new(pg_ctl)
        .arg("-D")
        .arg(data)
        .args(["-m", "fast", "-w", "stop"])
        .output()?;
    if output.status.success() {
        return Ok(());
    }
    // Другой процесс мог остановить сервер между status и stop.
    if !is_running(pg_ctl, data)? {
        return Ok(());
    }
    Err(failure("pg_ctl stop", &output))
}
