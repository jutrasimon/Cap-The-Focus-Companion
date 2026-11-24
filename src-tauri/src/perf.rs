//! Mesure de ce que Cap coûte : processeur et mémoire privée de Cap et de ses processus enfants
//! (moteur d'affichage WebView2). Appels Windows directs, sans bibliothèque lourde.

#[derive(Default, Clone)]
pub struct Sample {
    /// % du processeur total (tous les cœurs)
    pub cpu: f64,
    pub cap_cpu: f64,
    /// Mémoire privée, en octets (non partagée : la vraie empreinte)
    pub private: u64,
    pub cap_private: u64,
    /// Mémoire de travail (inclut le partagé, donc surestimée)
    pub working: u64,
    pub procs: usize,
    /// Mémoire privée en RAM (ce que montre le Gestionnaire des tâches)
    pub ws: u64,
    pub cap_ws: u64,
    pub list: Vec<ProcMem>,
}

#[derive(Clone, Debug)]
pub struct ProcMem {
    pub pid: u32,
    /// Mémoire privée en RAM
    pub ws: u64,
    /// Mémoire privée réservée (inclut ce qui est sur le disque)
    pub private: u64,
}

#[cfg(windows)]
mod imp {
    use super::{ProcMem, Sample};
    use std::collections::HashMap;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX2};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    pub struct Probe {
        tree: Vec<u32>,
        tree_at: std::time::Instant,
        last: HashMap<u32, u64>,
        last_at: Option<std::time::Instant>,
        cores: f64,
    }

    impl Probe {
        pub fn new() -> Self {
            Probe {
                tree: vec![],
                tree_at: std::time::Instant::now() - std::time::Duration::from_secs(60),
                last: HashMap::new(),
                last_at: None,
                cores: std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f64,
            }
        }

        /// Cap + tous ses descendants
        fn descendants(root: u32) -> Vec<u32> {
            let mut pairs = vec![];
            unsafe {
                let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
                if snap == INVALID_HANDLE_VALUE {
                    return vec![root];
                }
                let mut e: PROCESSENTRY32W = std::mem::zeroed();
                e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                if Process32FirstW(snap, &mut e) != 0 {
                    loop {
                        pairs.push((e.th32ProcessID, e.th32ParentProcessID));
                        if Process32NextW(snap, &mut e) == 0 {
                            break;
                        }
                    }
                }
                CloseHandle(snap);
            }
            let mut tree = vec![root];
            loop {
                let add: Vec<u32> = pairs.iter().filter(|(p, pp)| !tree.contains(p) && tree.contains(pp)).map(|(p, _)| *p).collect();
                if add.is_empty() {
                    break;
                }
                tree.extend(add);
            }
            tree
        }

        pub fn sample(&mut self) -> Sample {
            let me = std::process::id();
            if self.tree_at.elapsed().as_secs() >= 10 {
                self.tree = Self::descendants(me);
                self.tree_at = std::time::Instant::now();
            }
            let now = std::time::Instant::now();
            let wall = self.last_at.map(|t| now.duration_since(t).as_secs_f64()).unwrap_or(0.0);
            let mut s = Sample::default();
            let mut next = HashMap::new();
            for &pid in &self.tree {
                unsafe {
                    let h: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
                    if h.is_null() {
                        continue;
                    }
                    let mut pmc: PROCESS_MEMORY_COUNTERS_EX2 = std::mem::zeroed();
                    pmc.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;
                    let mem_ok = K32GetProcessMemoryInfo(h, &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, pmc.cb) != 0;
                    let (mut c, mut e, mut k, mut u): (FILETIME, FILETIME, FILETIME, FILETIME) = std::mem::zeroed();
                    let t_ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) != 0;
                    CloseHandle(h);
                    s.procs += 1;
                    if mem_ok {
                        s.private += pmc.PrivateUsage as u64;
                        s.working += pmc.WorkingSetSize as u64;
                        s.ws += pmc.PrivateWorkingSetSize as u64;
                        s.list.push(ProcMem { pid, ws: pmc.PrivateWorkingSetSize as u64, private: pmc.PrivateUsage as u64 });
                        if pid == me {
                            s.cap_private = pmc.PrivateUsage as u64;
                            s.cap_ws = pmc.PrivateWorkingSetSize as u64;
                        }
                    }
                    if t_ok {
                        let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
                        let total = ft(k) + ft(u); // centaines de nanosecondes
                        if let (Some(prev), true) = (self.last.get(&pid), wall > 0.0) {
                            let pct = (total.saturating_sub(*prev) as f64 / 1e7) / wall / self.cores * 100.0;
                            s.cpu += pct;
                            if pid == me {
                                s.cap_cpu = pct;
                            }
                        }
                        next.insert(pid, total);
                    }
                }
            }
            s.list.sort_by(|a, b| b.ws.cmp(&a.ws));
            self.last = next;
            self.last_at = Some(now);
            s
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Sample;
    pub struct Probe;
    impl Probe {
        pub fn new() -> Self {
            Probe
        }
        pub fn sample(&mut self) -> Sample {
            Sample { procs: 1, ..Default::default() }
        }
    }
}

pub use imp::Probe;
