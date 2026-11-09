use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use interprocess::local_socket::{Name, Stream, prelude::*};

#[cfg(unix)]
use interprocess::local_socket::GenericFilePath;
#[cfg(windows)]
use interprocess::local_socket::GenericNamespaced;

use crate::{
    config::Config,
    protocol::{MAX_MESSAGE_BYTES, ProgressEvent, Request, Response},
};

// v9 requires automatic GPU selection and backend status reporting.
// Older daemons must not silently ignore those fields.
const ENDPOINT_FILE: &str = "shellai-v9.sock";

pub fn endpoint_name() -> Result<Name<'static>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let directory = Config::cache_dir()?;
        std::fs::create_dir_all(&directory)
            .with_context(|| format!("failed to create {}", directory.display()))?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to secure {}", directory.display()))?;
        Ok(directory
            .join(ENDPOINT_FILE)
            .to_fs_name::<GenericFilePath>()?
            .into_owned())
    }

    #[cfg(windows)]
    {
        use std::hash::{DefaultHasher, Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        Config::cache_dir()?.hash(&mut hasher);
        let name = format!("shellai-v9-{:x}", hasher.finish());
        anyhow::ensure!(
            GenericNamespaced::is_supported(),
            "Windows named pipes are unavailable"
        );
        Ok(name.to_ns_name::<GenericNamespaced>()?.into_owned())
    }
}

pub fn request(request: &Request, cold_start: bool) -> Result<Response> {
    match connect_and_send(request) {
        Ok(response) => Ok(response),
        Err(first_error) if cold_start => {
            spawn_server().context("failed to cold-start the shellai server")?;
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut last_error = first_error;
            while Instant::now() < deadline {
                thread::sleep(Duration::from_millis(50));
                match connect_and_send(request) {
                    Ok(response) => return Ok(response),
                    Err(error) => last_error = error,
                }
            }
            Err(last_error)
                .context("the cold-started server did not become ready within 10 seconds")
        }
        Err(error) => Err(error).context("the shellai server is not running"),
    }
}

fn connect_and_send(request: &Request) -> Result<Response> {
    let mut connection = BufReader::new(
        Stream::connect(endpoint_name()?).context("could not connect to the local endpoint")?,
    );
    let mut encoded = serde_json::to_vec(request)?;
    anyhow::ensure!(encoded.len() <= MAX_MESSAGE_BYTES, "request is too large");
    encoded.push(b'\n');
    connection.get_mut().write_all(&encoded)?;
    connection.get_mut().flush()?;

    read_response(&mut connection, &mut |event| {
        eprintln!("{}", event.display())
    })
}

fn read_response(
    reader: &mut impl BufRead,
    progress: &mut dyn FnMut(ProgressEvent),
) -> Result<Response> {
    loop {
        let mut line = String::new();
        (&mut *reader)
            .take((MAX_MESSAGE_BYTES + 1) as u64)
            .read_line(&mut line)?;
        anyhow::ensure!(
            !line.is_empty(),
            "server closed the connection without a response"
        );
        anyhow::ensure!(
            line.len() <= MAX_MESSAGE_BYTES,
            "server response is too large"
        );
        if let Ok(event) = serde_json::from_str::<ProgressEvent>(&line) {
            progress(event);
            continue;
        }
        return serde_json::from_str(&line).context("server returned an invalid response");
    }
}

fn spawn_server() -> Result<()> {
    let executable = std::env::current_exe().context("could not locate the shellai executable")?;
    let mut command = Command::new(executable);
    command
        .arg("server")
        .arg("--background-child")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    command.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn progress_frames_precede_success_or_failure() {
        for response in [
            Response::success(Some("pwd".into()), true, None),
            Response::failure("download failed", false, None),
        ] {
            let event = ProgressEvent::Download {
                filename: "model.gguf".into(),
                downloaded: 50,
                total: 100,
                finished: false,
            };
            let frames = format!(
                "{}\n{}\n",
                serde_json::to_string(&event).unwrap(),
                serde_json::to_string(&response).unwrap()
            );
            let mut events = Vec::new();
            let received = read_response(&mut std::io::Cursor::new(frames), &mut |event| {
                events.push(event)
            })
            .unwrap();
            assert_eq!(received.ok, response.ok);
            assert_eq!(received.command, response.command);
            assert_eq!(events.len(), 1);
            assert!(events[0].display().contains("50%"));
        }
    }

    #[test]
    fn truncated_progress_stream_is_an_error() {
        let mut reader = std::io::Cursor::new(
            "{\"type\":\"download\",\"filename\":\"m\",\"downloaded\":0,\"total\":10,\"finished\":false}\n",
        );
        assert!(read_response(&mut reader, &mut |_| {}).is_err());
    }

    #[test]
    fn protocol_endpoint_is_versioned() {
        assert_ne!(ENDPOINT_FILE, "shellai.sock");
        assert_ne!(ENDPOINT_FILE, "shellai-v5.sock");
        assert_ne!(ENDPOINT_FILE, "shellai-v6.sock");
        assert_eq!(ENDPOINT_FILE, "shellai-v9.sock");
    }
}
