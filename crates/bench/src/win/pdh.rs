//! A small safe wrapper over Performance Data Helper queries, for the
//! counters that only PDH exposes (GPU engines, GPU memory, disks).

use anyhow::{bail, Result};
use windows::core::PCWSTR;
use windows::Win32::System::Performance::PdhOpenQueryW;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PDH_FMT,
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA,
};

/// Lets utilisation counters go over 100 (e.g. summed engines).
const PDH_FMT_NOCAP100: u32 = 0x8000;
const PDH_CSTATUS_INVALID: u32 = 0x8000_0000;

pub struct Query {
    handle: PDH_HQUERY,
}

#[derive(Debug, Clone, Copy)]
pub struct Counter(PDH_HCOUNTER);

impl Query {
    pub fn open() -> Result<Self> {
        let mut handle = PDH_HQUERY::default();
        // SAFETY: `handle` is a valid out-pointer; a null data source means
        // live data. The handle is closed in Drop.
        let status = unsafe { PdhOpenQueryW(PCWSTR::null(), 0, &mut handle) };
        check(status, "PdhOpenQueryW")?;
        Ok(Self { handle })
    }

    /// Adds a counter by its English path, e.g. `\GPU Engine(*)\Utilization
    /// Percentage`. Wildcard instances are re-expanded on every collect, so
    /// processes that start later still show up.
    pub fn add(&self, path: &str) -> Result<Counter> {
        let wide: Vec<u16> = path.encode_utf16().chain([0]).collect();
        let mut counter = PDH_HCOUNTER::default();
        // SAFETY: `wide` is NUL-terminated and outlives the call; `counter`
        // is a valid out-pointer; the query handle is open.
        let status = unsafe { PdhAddEnglishCounterW(self.handle, PCWSTR(wide.as_ptr()), 0, &mut counter) };
        check(status, path)?;
        Ok(Counter(counter))
    }

    pub fn collect(&self) -> Result<()> {
        // SAFETY: the query handle is open.
        check(unsafe { PdhCollectQueryData(self.handle) }, "PdhCollectQueryData")
    }

    /// Every instance's value as `(instance name, value)`. Rate counters
    /// need two collects before they have values.
    pub fn values(&self, counter: Counter) -> Result<Vec<(String, f64)>> {
        let format = PDH_FMT(PDH_FMT_DOUBLE.0 | PDH_FMT_NOCAP100);
        let (mut size, mut count) = (0u32, 0u32);
        // SAFETY: a sizing call: no buffer, just valid out-pointers.
        let status = unsafe { PdhGetFormattedCounterArrayW(counter.0, format, &mut size, &mut count, None) };
        if status != PDH_MORE_DATA {
            check(status, "PdhGetFormattedCounterArrayW")?;
            return Ok(Vec::new());
        }
        // u64s keep the items 8-byte aligned; the names are stored after them.
        let mut buf = vec![0u64; (size as usize).div_ceil(8)];
        let items = buf.as_mut_ptr().cast::<PDH_FMT_COUNTERVALUE_ITEM_W>();
        // SAFETY: `buf` holds at least `size` bytes, aligned for the items.
        let status = unsafe { PdhGetFormattedCounterArrayW(counter.0, format, &mut size, &mut count, Some(items)) };
        check(status, "PdhGetFormattedCounterArrayW")?;

        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count as usize {
            // SAFETY: PDH wrote `count` items at the start of `buf`, and each
            // name points to a NUL-terminated string inside `buf`, which is
            // still alive. A valid status means the double is the set field.
            unsafe {
                let item = &*items.add(i);
                if item.FmtValue.CStatus & PDH_CSTATUS_INVALID != 0 {
                    continue;
                }
                out.push((item.szName.to_string().unwrap_or_default(), item.FmtValue.Anonymous.doubleValue));
            }
        }
        Ok(out)
    }

    /// The sum over all instances (for `_Total`-style counters, the value).
    pub fn sum(&self, counter: Counter) -> Result<f64> {
        Ok(self.values(counter)?.iter().map(|(_, v)| v).sum())
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        // SAFETY: we own the open query and close it once; this also frees
        // its counters.
        unsafe { PdhCloseQuery(self.handle) };
    }
}

// SAFETY: a PDH query handle can be used from any thread, as long as calls
// don't overlap; `Query` has no `&self` methods that run concurrently
// because the sampler owns it on one thread.
unsafe impl Send for Query {}

fn check(status: u32, what: &str) -> Result<()> {
    if status != 0 {
        bail!("{what} failed: PDH status 0x{status:08X}");
    }
    Ok(())
}
