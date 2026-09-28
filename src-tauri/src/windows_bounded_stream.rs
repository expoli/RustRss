//! A size-limited IStream for WebView2's PNG CapturePreview output.
//!
//! WebView2 writes into the supplied stream before invoking its completion
//! handler. Checking the size only in that handler permits an unbounded HGLOBAL
//! allocation, so all writes and resizes must pass through this wrapper.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use windows::{
    core::{implement, Error, Ref, Result, HRESULT},
    Win32::{
        Foundation::{E_ABORT, STG_E_INVALIDFUNCTION, STG_E_MEDIUMFULL},
        System::Com::{
            ISequentialStream_Impl, IStream, IStream_Impl, LOCKTYPE, STATFLAG, STATFLAG_NONAME,
            STATSTG, STGC, STREAM_SEEK, STREAM_SEEK_CUR, STREAM_SEEK_END, STREAM_SEEK_SET,
        },
    },
};

#[implement(IStream)]
struct BoundedStream {
    inner: Mutex<IStream>,
    limit: u64,
    cancelled: Arc<AtomicBool>,
    exceeded: Arc<AtomicBool>,
    on_exceeded: Box<dyn Fn() + Send + Sync>,
}

pub fn wrap(
    inner: IStream,
    max_bytes: usize,
    cancelled: Arc<AtomicBool>,
    on_exceeded: impl Fn() + Send + Sync + 'static,
) -> (IStream, Arc<AtomicBool>) {
    let exceeded = Arc::new(AtomicBool::new(false));
    let stream = BoundedStream {
        inner: Mutex::new(inner),
        limit: max_bytes as u64,
        cancelled,
        exceeded: exceeded.clone(),
        on_exceeded: Box::new(on_exceeded),
    }
    .into();
    (stream, exceeded)
}

impl BoundedStream_Impl {
    fn active(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err(Error::from(E_ABORT))
        } else {
            Ok(())
        }
    }

    fn within_limit(&self, end: Option<u64>) -> Result<()> {
        if end.is_none_or(|end| end > self.limit) {
            if !self.exceeded.swap(true, Ordering::AcqRel) {
                (self.on_exceeded)();
            }
            Err(Error::from(STG_E_MEDIUMFULL))
        } else {
            Ok(())
        }
    }
}

impl ISequentialStream_Impl for BoundedStream_Impl {
    fn Read(&self, pv: *mut std::ffi::c_void, cb: u32, pcbread: *mut u32) -> HRESULT {
        if let Err(error) = self.active() {
            return error.code();
        }
        unsafe { self.inner.lock().unwrap().Read(pv, cb, Some(pcbread)) }
    }

    fn Write(&self, pv: *const std::ffi::c_void, cb: u32, pcbwritten: *mut u32) -> HRESULT {
        if !pcbwritten.is_null() {
            unsafe { *pcbwritten = 0 };
        }
        if let Err(error) = self.active() {
            return error.code();
        }
        let inner = self.inner.lock().unwrap();
        let mut position = 0;
        if let Err(error) = unsafe { inner.Seek(0, STREAM_SEEK_CUR, Some(&mut position)) } {
            return error.code();
        }
        if let Err(error) = self.within_limit(position.checked_add(u64::from(cb))) {
            return error.code();
        }
        // The cancellation guard may be dropped while waiting for this lock.
        if let Err(error) = self.active() {
            return error.code();
        }
        unsafe { inner.Write(pv, cb, Some(pcbwritten)) }
    }
}

impl IStream_Impl for BoundedStream_Impl {
    fn Seek(&self, move_by: i64, origin: STREAM_SEEK, new_position: *mut u64) -> Result<()> {
        self.active()?;
        let inner = self.inner.lock().unwrap();
        let base = if origin == STREAM_SEEK_SET {
            0
        } else if origin == STREAM_SEEK_CUR {
            let mut position = 0;
            unsafe { inner.Seek(0, STREAM_SEEK_CUR, Some(&mut position)) }?;
            position
        } else if origin == STREAM_SEEK_END {
            let mut stat = STATSTG::default();
            unsafe { inner.Stat(&mut stat, STATFLAG_NONAME) }?;
            stat.cbSize
        } else {
            return Err(Error::from(STG_E_INVALIDFUNCTION));
        };
        let position = i128::from(base) + i128::from(move_by);
        if position < 0 {
            return Err(Error::from(STG_E_INVALIDFUNCTION));
        }
        self.within_limit(u64::try_from(position).ok())?;
        unsafe { inner.Seek(move_by, origin, Some(new_position)) }
    }

    fn SetSize(&self, new_size: u64) -> Result<()> {
        self.active()?;
        self.within_limit(Some(new_size))?;
        unsafe { self.inner.lock().unwrap().SetSize(new_size) }
    }

    fn CopyTo(
        &self,
        destination: Ref<'_, IStream>,
        count: u64,
        read: *mut u64,
        written: *mut u64,
    ) -> Result<()> {
        self.active()?;
        unsafe {
            self.inner
                .lock()
                .unwrap()
                .CopyTo(destination.ok()?, count, Some(read), Some(written))
        }
    }

    fn Commit(&self, flags: &STGC) -> Result<()> {
        self.active()?;
        unsafe { self.inner.lock().unwrap().Commit(*flags) }
    }

    fn Revert(&self) -> Result<()> {
        self.active()?;
        unsafe { self.inner.lock().unwrap().Revert() }
    }

    fn LockRegion(&self, offset: u64, count: u64, kind: &LOCKTYPE) -> Result<()> {
        self.active()?;
        unsafe { self.inner.lock().unwrap().LockRegion(offset, count, *kind) }
    }

    fn UnlockRegion(&self, offset: u64, count: u64, kind: u32) -> Result<()> {
        self.active()?;
        unsafe { self.inner.lock().unwrap().UnlockRegion(offset, count, kind) }
    }

    fn Stat(&self, stat: *mut STATSTG, flags: &STATFLAG) -> Result<()> {
        self.active()?;
        unsafe { self.inner.lock().unwrap().Stat(stat, *flags) }
    }

    fn Clone(&self) -> Result<IStream> {
        // Cloning the HGLOBAL stream would hand the caller an uncapped writer.
        Err(Error::from(STG_E_INVALIDFUNCTION))
    }
}
