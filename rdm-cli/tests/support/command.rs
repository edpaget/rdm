//! Bounded subprocess capture for hermetic integration fixtures.
use std::io::{self, Read};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        #[cfg(unix)]
        let _ = Command::new("kill")
            .args(["-KILL", "--", &format!("-{}", self.0.id())])
            .stderr(Stdio::null())
            .status();
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

pub fn bounded_output(command: &mut Command) -> io::Result<Output> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut process = Process(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?,
    );
    let mut stdout = process.0.stdout.take().unwrap();
    let mut stderr = process.0.stderr.take().unwrap();
    let (out_tx, out) = mpsc::channel();
    let (err_tx, err) = mpsc::channel();
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let _ = out_tx.send(stdout.read_to_end(&mut data).map(|_| data));
    });
    std::thread::spawn(move || {
        let mut data = Vec::new();
        let _ = err_tx.send(stderr.read_to_end(&mut data).map(|_| data));
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    let timeout = || {
        io::Error::new(
            io::ErrorKind::TimedOut,
            format!("fixture command exceeded 20s: {command:?}"),
        )
    };
    let status = loop {
        if let Some(status) = process.0.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            return Err(timeout());
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Ok(Output {
        status,
        stdout: out
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| timeout())??,
        stderr: err
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| timeout())??,
    })
}
