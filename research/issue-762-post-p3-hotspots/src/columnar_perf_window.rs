//! #814：只复制到冻结 PMU 导出树；控制握手位于公共 step 墙钟之外。
use std::{
    fs::{File, OpenOptions},
    io::{self, BufRead, BufReader, Write},
    sync::Mutex,
};

struct Controller {
    command: File,
    acknowledgement: BufReader<File>,
    enabled: bool,
    calibration: bool,
}

impl Controller {
    fn open() -> io::Result<Self> {
        let path = |key| {
            std::env::var_os(key)
                .filter(|p| !p.is_empty())
                .ok_or_else(|| io::Error::other(format!("missing {key}")))
        };
        let command = OpenOptions::new()
            .write(true)
            .open(path("LF814_PERF_CONTROL")?)?;
        let acknowledgement = BufReader::new(File::open(path("LF814_PERF_ACK")?)?);
        let calibration = match std::env::var("LF814_PERF_CALIBRATE").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("0") => false,
            Ok("1") => true,
            _ => return Err(io::Error::other("invalid calibration mode")),
        };
        Ok(Self {
            command,
            acknowledgement,
            enabled: false,
            calibration,
        })
    }

    fn set(&mut self, enabled: bool) -> io::Result<()> {
        if enabled == self.enabled {
            return Err(io::Error::other("perf window transition repeated"));
        }
        self.command
            .write_all(if enabled { b"enable\n" } else { b"disable\n" })?;
        self.command.flush()?;
        let mut reply = String::new();
        self.acknowledgement.read_line(&mut reply)?;
        if reply != "ack\n" {
            return Err(io::Error::other("perf control acknowledgement missing"));
        }
        self.enabled = enabled;
        Ok(())
    }
}

static CONTROL: Mutex<Option<Controller>> = Mutex::new(None);

fn with_controller<R>(run: impl FnOnce(&mut Controller) -> io::Result<R>) -> io::Result<R> {
    let mut slot = CONTROL
        .lock()
        .map_err(|_| io::Error::other("perf controller poisoned"))?;
    if slot.is_none() {
        *slot = Some(Controller::open()?);
    }
    run(slot
        .as_mut()
        .ok_or_else(|| io::Error::other("perf controller absent"))?)
}

pub(super) fn before_step() -> io::Result<bool> {
    with_controller(|controller| {
        controller.set(true)?;
        if controller.calibration {
            controller.set(false)?;
        }
        Ok(controller.calibration)
    })
}

pub(super) fn after_step(calibration: bool) -> io::Result<()> {
    with_controller(|controller| {
        if calibration != controller.calibration {
            return Err(io::Error::other("perf calibration mode changed"));
        }
        if !calibration {
            controller.set(false)?;
        }
        Ok(())
    })
}
