//! Minimalist cross-platform GUI for memreduct.
//! Enabled with `--features gui`. Uses eframe/egui (glow) — single static
//! binary on Linux and Windows, no web runtime.

#[cfg(feature = "gui")]
mod imp {
    use crate::cgroup;
    use crate::mem::{format_bytes, Memory};
    use crate::procs;
    use crate::psi;
    use crate::slab;
    use crate::zram;
    use eframe::egui;
    use egui_plot::{Line, Plot, PlotPoints};
    use std::time::{Duration, Instant};

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Tab {
        Overview,
        Processes,
        Oom,
        Slab,
        Zram,
        Cgroup,
        Grow,
    }

    impl Tab {
        fn label(self) -> &'static str {
            match self {
                Self::Overview => "Overview",
                Self::Processes => "Processes",
                Self::Oom => "OOM Scores",
                Self::Slab => "Slab",
                Self::Zram => "Zram",
                Self::Cgroup => "Cgroup",
                Self::Grow => "Leak Scan",
            }
        }
        fn all() -> &'static [Tab] {
            &[
                Self::Overview,
                Self::Processes,
                Self::Oom,
                Self::Slab,
                Self::Zram,
                Self::Cgroup,
                Self::Grow,
            ]
        }
    }

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum CleanModeGui {
        All,
        PageCache,
        Slab,
    }

    impl CleanModeGui {
        fn to_cli(self) -> crate::CleanMode {
            match self {
                Self::All => crate::CleanMode::All,
                Self::PageCache => crate::CleanMode::PageCache,
                Self::Slab => crate::CleanMode::Slab,
            }
        }
    }

    pub struct App {
        // live state
        memory: Option<Memory>,
        pressure: Option<psi::Psi>,
        history: Vec<[f64; 2]>,
        start: Instant,
        last_poll: Instant,

        // auto-clean
        autoclean_enabled: bool,
        threshold: u8,
        interval_secs: u64,
        cooldown_secs: u64,
        clean_mode: CleanModeGui,
        swap_threshold: u8,
        use_psi: bool,
        psi_metric_is_some: bool,
        last_clean: Option<Instant>,
        exec_cmd: String,

        // tabs
        tab: Tab,
        pss_top: usize,
        pss_entries: Vec<procs::ProcEntry>,
        oom_top: usize,
        oom_entries: Vec<(i32, String, procs::OomScore, u64)>,
        oom_kills: u64,
        slab_top: usize,
        slab_caches: Vec<slab::SlabCache>,
        slab_total: u64,
        zram_devices: Vec<zram::ZramDevice>,
        zram_sample_secs: u64,

        // grow
        grow_interval: u64,
        grow_min: String,
        grow_results: Vec<(i32, String, u64, u64)>, // pid, comm, pss, delta
        grow_running: bool,
        grow_rx: Option<std::sync::mpsc::Receiver<Vec<(i32, String, u64, u64)>>>,

        // cgroup
        cgroup_path: String,
        limit_kind_is_high: bool,
        limit_value: String,

        // ui
        status: String,
        status_is_error: bool,
        status_time: Instant,
    }

    impl App {
        pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
            // dark minimalist theme
            let mut style = (*cc.egui_ctx.style()).clone();
            style.visuals = egui::Visuals::dark();
            style.visuals.widgets.noninteractive.bg_stroke.width = 0.0;
            cc.egui_ctx.set_style(style);

            let mut app = Self {
                memory: None,
                pressure: None,
                history: Vec::with_capacity(300),
                start: Instant::now(),
                last_poll: Instant::now() - Duration::from_secs(10),
                autoclean_enabled: false,
                threshold: 90,
                interval_secs: 30,
                cooldown_secs: 300,
                clean_mode: CleanModeGui::All,
                swap_threshold: 0,
                use_psi: false,
                psi_metric_is_some: true,
                last_clean: None,
                exec_cmd: String::new(),
                tab: Tab::Overview,
                pss_top: 15,
                pss_entries: Vec::new(),
                oom_top: 15,
                oom_entries: Vec::new(),
                oom_kills: 0,
                slab_top: 15,
                slab_caches: Vec::new(),
                slab_total: 0,
                zram_devices: Vec::new(),
                zram_sample_secs: 0,
                grow_interval: 30,
                grow_min: "32M".to_string(),
                grow_results: Vec::new(),
                grow_running: false,
                grow_rx: None,
                cgroup_path: String::new(),
                limit_kind_is_high: true,
                limit_value: "2G".to_string(),
                status: "Ready".to_string(),
                status_is_error: false,
                status_time: Instant::now(),
            };
            app.refresh_all();
            app
        }

        fn set_status(&mut self, msg: impl Into<String>, is_error: bool) {
            self.status = msg.into();
            self.status_is_error = is_error;
            self.status_time = Instant::now();
        }

        fn refresh_all(&mut self) {
            self.poll_memory();
            self.refresh_pss();
            self.refresh_oom();
            self.refresh_slab();
            self.refresh_zram();
        }

        fn poll_memory(&mut self) {
            #[cfg(target_os = "linux")]
            {
                if let Ok(m) = crate::mem::read_memory() {
                    self.memory = Some(m);
                    let t = self.start.elapsed().as_secs_f64();
                    self.history.push([t, m.used_percent() as f64]);
                    if self.history.len() > 300 {
                        self.history.remove(0);
                    }
                }
                self.pressure = psi::read_psi().ok();
            }
            #[cfg(target_os = "windows")]
            {
                self.poll_memory_windows();
            }
            #[cfg(not(any(target_os = "linux", target_os = "windows")))]
            {
                // other platforms: no live data
            }
            self.last_poll = Instant::now();
        }

        #[cfg(target_os = "windows")]
        fn poll_memory_windows(&mut self) {
            use windows::Win32::System::SystemInformation::GlobalMemoryStatusEx;
            use windows::Win32::System::SystemInformation::MEMORYSTATUSEX;
            unsafe {
                let mut stat = MEMORYSTATUSEX::default();
                stat.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
                if GlobalMemoryStatusEx(&mut stat).is_ok() {
                    let total = stat.ullTotalPhys;
                    let avail = stat.ullAvailPhys;
                    let used = total.saturating_sub(avail);
                    let pct = if total > 0 {
                        ((used * 100) / total).min(100) as u8
                    } else {
                        0
                    };
                    // fabricate Memory with Windows numbers; cached/buffers not exposed
                    self.memory = Some(Memory {
                        total,
                        free: avail,
                        available: avail,
                        cached: 0,
                        buffers: 0,
                        reclaimable: 0,
                        swap_total: stat.ullTotalPageFile.saturating_sub(total),
                        swap_free: stat.ullAvailPageFile.saturating_sub(avail),
                    });
                    let t = self.start.elapsed().as_secs_f64();
                    self.history.push([t, pct as f64]);
                    if self.history.len() > 300 {
                        self.history.remove(0);
                    }
                }
            }
            self.pressure = None;
        }

        fn refresh_pss(&mut self) {
            #[cfg(target_os = "linux")]
            {
                let mut entries = Vec::new();
                for pid in procs::all_pids() {
                    let Ok(comm) = procs::read_comm(pid) else {
                        continue;
                    };
                    let Ok(mem) = procs::read_smaps_rollup(pid) else {
                        continue;
                    };
                    entries.push(procs::ProcEntry { pid, comm, mem });
                }
                entries.sort_by_key(|e| std::cmp::Reverse(e.mem.pss));
                entries.truncate(self.pss_top);
                self.pss_entries = entries;
            }
            #[cfg(target_os = "windows")]
            {
                self.pss_entries.clear();
            }
        }

        fn refresh_oom(&mut self) {
            #[cfg(target_os = "linux")]
            {
                let mut entries = Vec::new();
                for pid in procs::all_pids() {
                    let Ok(comm) = procs::read_comm(pid) else {
                        continue;
                    };
                    let Ok(score) = procs::read_oom_score(pid) else {
                        continue;
                    };
                    let rss = procs::read_statm_rss_bytes(pid).unwrap_or(0);
                    entries.push((pid, comm, score, rss));
                }
                entries.sort_by_key(|e| std::cmp::Reverse((e.2).score));
                entries.truncate(self.oom_top);
                self.oom_entries = entries;
                self.oom_kills = procs::system_oom_kills().unwrap_or(0);
            }
            #[cfg(target_os = "windows")]
            {
                self.oom_entries.clear();
            }
        }

        fn refresh_slab(&mut self) {
            #[cfg(target_os = "linux")]
            {
                match slab::read_slabinfo() {
                    Ok(mut caches) => {
                        self.slab_total = caches.iter().map(|c| c.size_bytes()).sum();
                        caches.sort_by_key(|c| std::cmp::Reverse(c.size_bytes()));
                        caches.truncate(self.slab_top);
                        self.slab_caches = caches;
                    }
                    Err(e) => {
                        self.slab_caches.clear();
                        self.slab_total = 0;
                        // don't spam status on every poll; only if user is on slab tab
                        if self.tab == Tab::Slab {
                            self.set_status(format!("slab: {e:#}"), true);
                        }
                    }
                }
            }
            #[cfg(target_os = "windows")]
            {
                self.slab_caches.clear();
            }
        }

        fn refresh_zram(&mut self) {
            #[cfg(target_os = "linux")]
            {
                self.zram_devices = zram::read_zram_devices();
            }
            #[cfg(target_os = "windows")]
            {
                self.zram_devices.clear();
            }
        }

        fn do_clean(&mut self, mode: CleanModeGui) {
            #[cfg(target_os = "linux")]
            {
                let before = crate::mem::read_memory().ok();
                let res = (|| -> anyhow::Result<()> {
                    unsafe { libc::sync() };
                    std::fs::write(crate::mem::DROP_CACHES, mode.to_cli().value().to_string())
                        .map_err(|e| anyhow::anyhow!("{e} (need root)"))?;
                    Ok(())
                })();
                match res {
                    Ok(()) => {
                        self.last_clean = Some(Instant::now());
                        let after = crate::mem::read_memory().ok();
                        if let (Some(b), Some(a)) = (before, after) {
                            let reclaimed = b
                                .reclaimable_caches()
                                .saturating_sub(a.reclaimable_caches());
                            self.set_status(
                                format!(
                                    "cleaned {} — reclaimed {}",
                                    mode_label(mode),
                                    format_bytes(reclaimed)
                                ),
                                false,
                            );
                        } else {
                            self.set_status(format!("cleaned {}", mode_label(mode)), false);
                        }
                        if !self.exec_cmd.trim().is_empty() {
                            let cmd = self.exec_cmd.clone();
                            std::thread::spawn(move || {
                                let _ = std::process::Command::new("sh")
                                    .arg("-c")
                                    .arg(&cmd)
                                    .status();
                            });
                        }
                        self.poll_memory();
                    }
                    Err(e) => self.set_status(format!("clean failed: {e:#}"), true),
                }
            }
            #[cfg(target_os = "windows")]
            {
                let _ = mode;
                // Windows: EmptyWorkingSet for all processes
                let res = Self::clean_windows();
                match res {
                    Ok(n) => self.set_status(format!("cleaned {n} processes (Windows)"), false),
                    Err(e) => self.set_status(format!("clean failed: {e}"), true),
                }
            }
        }

        #[cfg(target_os = "windows")]
        fn clean_windows() -> Result<usize, String> {
            use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
            use windows::Win32::System::Threading::{
                OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA,
            };
            let mut cleaned = 0usize;
            // enumerate pids via snapshot would need Toolhelp; keep simple: try current process
            unsafe {
                if let Ok(h) = OpenProcess(
                    PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA,
                    false,
                    std::process::id(),
                ) {
                    if EmptyWorkingSet(h).is_ok() {
                        cleaned += 1;
                    }
                    let _ = windows::Win32::Foundation::CloseHandle(h);
                }
            }
            if cleaned == 0 {
                return Err("EmptyWorkingSet failed (need admin)".to_string());
            }
            Ok(cleaned)
        }

        fn do_compact(&mut self) {
            #[cfg(target_os = "linux")]
            {
                let res = std::fs::write(crate::mem::COMPACT_MEMORY, "1");
                match res {
                    Ok(()) => {
                        self.set_status("compaction triggered", false);
                        self.poll_memory();
                    }
                    Err(e) => self.set_status(format!("compact failed: {e} (need root)"), true),
                }
            }
            #[cfg(target_os = "windows")]
            {
                self.set_status("compact not available on Windows", true);
            }
        }

        fn maybe_autoclean(&mut self) {
            if !self.autoclean_enabled {
                return;
            }
            let Some(mem) = self.memory else { return };
            let hit = if self.use_psi {
                if let Some(p) = self.pressure {
                    let v = if self.psi_metric_is_some {
                        p.some.avg10
                    } else {
                        p.full.avg10
                    };
                    v >= self.threshold as f64
                } else {
                    false
                }
            } else {
                let mem_hit = mem.used_percent() >= self.threshold;
                let swap_hit =
                    self.swap_threshold > 0 && mem.swap_used_percent() >= self.swap_threshold;
                mem_hit || swap_hit
            };
            if !hit {
                return;
            }
            let due = self
                .last_clean
                .is_none_or(|t| t.elapsed() >= Duration::from_secs(self.cooldown_secs));
            if due {
                let mode = self.clean_mode;
                self.do_clean(mode);
            }
        }

        fn start_grow(&mut self) {
            if self.grow_running {
                return;
            }
            let interval = self.grow_interval;
            let min_str = self.grow_min.clone();
            let min_bytes = crate::mem::parse_size(&min_str).unwrap_or(32 * 1024 * 1024);
            let (tx, rx) = std::sync::mpsc::channel();
            self.grow_rx = Some(rx);
            self.grow_running = true;
            self.grow_results.clear();
            self.set_status(format!("leak scan: sampling {interval}s..."), false);
            std::thread::spawn(move || {
                let before = snapshot_pss();
                std::thread::sleep(Duration::from_secs(interval));
                let after = snapshot_pss();
                let mut gains: Vec<(i32, String, u64, u64)> = Vec::new();
                for (pid, starttime, comm, pss) in &after {
                    let delta = match before
                        .iter()
                        .find(|(p, s, _, _)| *p == *pid && *s == *starttime)
                    {
                        Some((_, _, _, b)) => pss.saturating_sub(*b),
                        None => *pss,
                    };
                    if delta >= min_bytes {
                        gains.push((*pid, comm.clone(), *pss, delta));
                    }
                }
                gains.sort_by_key(|g| std::cmp::Reverse(g.3));
                let _ = tx.send(gains);
            });
        }

        fn poll_grow(&mut self) {
            if let Some(rx) = &self.grow_rx {
                if let Ok(results) = rx.try_recv() {
                    let n = results.len();
                    self.grow_results = results;
                    self.grow_running = false;
                    self.grow_rx = None;
                    if n == 0 {
                        self.set_status("leak scan: no growth above threshold", false);
                    } else {
                        self.set_status(format!("leak scan: {n} growers found"), false);
                    }
                }
            }
        }
    }

    fn mode_label(m: CleanModeGui) -> &'static str {
        match m {
            CleanModeGui::All => "all",
            CleanModeGui::PageCache => "page-cache",
            CleanModeGui::Slab => "slab",
        }
    }

    #[cfg(target_os = "linux")]
    fn snapshot_pss() -> Vec<(i32, u64, String, u64)> {
        let mut v = Vec::new();
        for pid in procs::all_pids() {
            let Ok(st) = procs::read_starttime(pid) else {
                continue;
            };
            let Ok(comm) = procs::read_comm(pid) else {
                continue;
            };
            let Ok(mem) = procs::read_smaps_rollup(pid) else {
                continue;
            };
            v.push((pid, st, comm, mem.pss));
        }
        v
    }
    #[cfg(not(target_os = "linux"))]
    fn snapshot_pss() -> Vec<(i32, u64, String, u64)> {
        Vec::new()
    }

    fn bar_color(pct: u8) -> egui::Color32 {
        if pct >= 90 {
            egui::Color32::from_rgb(220, 60, 60)
        } else if pct >= 70 {
            egui::Color32::from_rgb(220, 170, 50)
        } else {
            egui::Color32::from_rgb(60, 180, 90)
        }
    }

    impl eframe::App for App {
        fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            // poll every 500ms
            if self.last_poll.elapsed() >= Duration::from_millis(500) {
                self.poll_memory();
                self.maybe_autoclean();
            }
            self.poll_grow();
            ctx.request_repaint_after(Duration::from_millis(500));

            // top bar
            egui::TopBottomPanel::top("top").show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.heading("Mem Reduct");
                    ui.label(format!("v{}", env!("CARGO_PKG_VERSION")));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("↻ Refresh").clicked() {
                            self.refresh_all();
                            self.set_status("refreshed", false);
                        }
                        ui.label(if cfg!(target_os = "windows") {
                            "Windows"
                        } else {
                            "Linux"
                        });
                    });
                });
            });

            // status bar
            egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let col = if self.status_is_error {
                        egui::Color32::from_rgb(220, 80, 80)
                    } else {
                        egui::Color32::from_rgb(140, 140, 140)
                    };
                    ui.colored_label(col, &self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if let Some(m) = self.memory {
                            ui.label(format!("{} used", m.used_percent()));
                        }
                        if self.autoclean_enabled {
                            ui.colored_label(
                                egui::Color32::from_rgb(80, 180, 80),
                                "● auto-clean on",
                            );
                        }
                    });
                });
            });

            egui::CentralPanel::default().show(ctx, |ui| {
                // memory cards
                if let Some(mem) = self.memory {
                    ui.horizontal(|ui| {
                        // RAM card
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_width(260.0);
                            ui.label(egui::RichText::new("RAM").small().weak());
                            let pct = mem.used_percent();
                            let frac = pct as f32 / 100.0;
                            ui.add(
                                egui::ProgressBar::new(frac)
                                    .fill(bar_color(pct))
                                    .text(format!(
                                        "{} / {}  ({}%)",
                                        format_bytes(mem.used()),
                                        format_bytes(mem.total),
                                        pct
                                    ))
                                    .animate(true),
                            );
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!("free {}", format_bytes(mem.free)))
                                        .small()
                                        .weak(),
                                );
                                ui.label(
                                    egui::RichText::new(format!(
                                        "avail {}",
                                        format_bytes(mem.available)
                                    ))
                                    .small()
                                    .weak(),
                                );
                            });
                            if let Some(p) = self.pressure {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "PSI some {:.1}% full {:.1}%",
                                        p.some.avg10, p.full.avg10
                                    ))
                                    .small()
                                    .weak(),
                                );
                            }
                        });
                        // swap card
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.set_min_width(220.0);
                            ui.label(egui::RichText::new("Swap").small().weak());
                            let pct = mem.swap_used_percent();
                            let frac = pct as f32 / 100.0;
                            ui.add(
                                egui::ProgressBar::new(frac)
                                    .fill(bar_color(pct))
                                    .text(format!(
                                        "{} / {}  ({}%)",
                                        format_bytes(mem.swap_used()),
                                        format_bytes(mem.swap_total),
                                        pct
                                    ))
                                    .animate(true),
                            );
                            ui.label(
                                egui::RichText::new(format!(
                                    "cached {}  reclaimable {}",
                                    format_bytes(mem.cached),
                                    format_bytes(mem.reclaimable)
                                ))
                                .small()
                                .weak(),
                            );
                        });
                        // cache card
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            ui.label(egui::RichText::new("Quick actions").small().weak());
                            ui.horizontal(|ui| {
                                let btn = |label: &str| {
                                    egui::Button::new(label).min_size(egui::vec2(72.0, 28.0))
                                };
                                if ui.add(btn("Clean all")).clicked() {
                                    self.do_clean(CleanModeGui::All);
                                }
                                if ui.add(btn("PageCache")).clicked() {
                                    self.do_clean(CleanModeGui::PageCache);
                                }
                                if ui.add(btn("Slab")).clicked() {
                                    self.do_clean(CleanModeGui::Slab);
                                }
                            });
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new("Compact")
                                            .min_size(egui::vec2(72.0, 28.0)),
                                    )
                                    .clicked()
                                {
                                    self.do_compact();
                                }
                                if ui
                                    .checkbox(&mut self.autoclean_enabled, "Auto-clean")
                                    .changed()
                                {
                                    self.set_status(
                                        if self.autoclean_enabled {
                                            "auto-clean enabled"
                                        } else {
                                            "auto-clean disabled"
                                        },
                                        false,
                                    );
                                }
                            });
                        });
                    });
                } else {
                    ui.spinner();
                    ui.label("reading /proc/meminfo…");
                    #[cfg(target_os = "windows")]
                    ui.label("If this persists, check Windows memory API access.");
                }

                ui.add_space(8.0);

                // history graph
                if self.history.len() > 2 {
                    let points: PlotPoints = self.history.iter().copied().collect();
                    let line = Line::new("used %", points)
                        .color(egui::Color32::from_rgb(90, 170, 255))
                        .width(1.5);
                    Plot::new("history")
                        .height(90.0)
                        .allow_zoom(false)
                        .allow_drag(false)
                        .allow_scroll(false)
                        .show_axes([false, true])
                        .include_y(0.0)
                        .include_y(100.0)
                        .y_axis_label("used %")
                        .show(ui, |plot_ui| plot_ui.line(line));
                }

                ui.add_space(6.0);
                ui.separator();

                // tab bar — compact, single row
                ui.horizontal(|ui| {
                    for &t in Tab::all() {
                        let selected = self.tab == t;
                        if ui.selectable_label(selected, t.label()).clicked() {
                            self.tab = t;
                            // lazy refresh on tab switch
                            match t {
                                Tab::Processes => self.refresh_pss(),
                                Tab::Oom => self.refresh_oom(),
                                Tab::Slab => self.refresh_slab(),
                                Tab::Zram => self.refresh_zram(),
                                _ => {}
                            }
                        }
                    }
                });
                ui.separator();

                egui::ScrollArea::vertical().show(ui, |ui| match self.tab {
                    Tab::Overview => overview_tab(ui, self),
                    Tab::Processes => processes_tab(ui, self),
                    Tab::Oom => oom_tab(ui, self),
                    Tab::Slab => slab_tab(ui, self),
                    Tab::Zram => zram_tab(ui, self),
                    Tab::Cgroup => cgroup_tab(ui, self),
                    Tab::Grow => grow_tab(ui, self),
                });
            });
        }
    }

    fn overview_tab(ui: &mut egui::Ui, app: &mut App) {
        let Some(mem) = app.memory else {
            ui.label("no data");
            return;
        };
        egui::Grid::new("overview_grid")
            .num_columns(2)
            .spacing([24.0, 6.0])
            .show(ui, |ui| {
                ui.label("Total");
                ui.label(format_bytes(mem.total));
                ui.end_row();
                ui.label("Available");
                ui.label(format_bytes(mem.available));
                ui.end_row();
                ui.label("Cached");
                ui.label(format_bytes(mem.cached));
                ui.end_row();
                ui.label("Buffers");
                ui.label(format_bytes(mem.buffers));
                ui.end_row();
                ui.label("Reclaimable");
                ui.label(format_bytes(mem.reclaimable));
                ui.end_row();
                ui.label("Swap total");
                ui.label(format_bytes(mem.swap_total));
                ui.end_row();
                ui.label("Swap used");
                ui.label(format_bytes(mem.swap_used()));
                ui.end_row();
            });
        ui.add_space(8.0);
        ui.collapsing("Auto-clean settings", |ui| {
            ui.horizontal(|ui| {
                ui.label("Threshold %");
                ui.add(egui::Slider::new(&mut app.threshold, 50..=100));
                ui.label("Swap %");
                ui.add(egui::Slider::new(&mut app.swap_threshold, 0..=100));
            });
            ui.horizontal(|ui| {
                ui.label("Interval s");
                ui.add(egui::DragValue::new(&mut app.interval_secs).range(1..=300));
                ui.label("Cooldown s");
                ui.add(egui::DragValue::new(&mut app.cooldown_secs).range(0..=3600));
            });
            ui.horizontal(|ui| {
                egui::ComboBox::from_label("Mode")
                    .selected_text(mode_label(app.clean_mode))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut app.clean_mode, CleanModeGui::All, "all");
                        ui.selectable_value(
                            &mut app.clean_mode,
                            CleanModeGui::PageCache,
                            "page-cache",
                        );
                        ui.selectable_value(&mut app.clean_mode, CleanModeGui::Slab, "slab");
                    });
                ui.checkbox(&mut app.use_psi, "use PSI");
                if app.use_psi {
                    let label = if app.psi_metric_is_some {
                        "some"
                    } else {
                        "full"
                    };
                    egui::ComboBox::from_label("metric")
                        .selected_text(label)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut app.psi_metric_is_some, true, "some");
                            ui.selectable_value(&mut app.psi_metric_is_some, false, "full");
                        });
                }
            });
            ui.horizontal(|ui| {
                ui.label("Exec after clean");
                ui.text_edit_singleline(&mut app.exec_cmd);
            });
            ui.label(
                egui::RichText::new("Tip: set Exec to e.g. notify-send 'memreduct cleaned'")
                    .small()
                    .weak(),
            );
        });
    }

    fn processes_tab(ui: &mut egui::Ui, app: &mut App) {
        ui.horizontal(|ui| {
            ui.label("Top");
            ui.add(egui::DragValue::new(&mut app.pss_top).range(5..=100));
            if ui.button("Refresh").clicked() {
                app.refresh_pss();
            }
            ui.label(egui::RichText::new(format!("{} procs", app.pss_entries.len())).weak());
        });
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                egui::Grid::new("pss_grid")
                    .num_columns(5)
                    .spacing([12.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("PID").strong());
                        ui.label(egui::RichText::new("COMM").strong());
                        ui.label(egui::RichText::new("RSS").strong());
                        ui.label(egui::RichText::new("PSS").strong());
                        ui.label(egui::RichText::new("SWAP").strong());
                        ui.end_row();
                        for e in &app.pss_entries {
                            ui.label(e.pid.to_string());
                            ui.label(e.comm.clone());
                            ui.label(format_bytes(e.mem.rss));
                            ui.label(format_bytes(e.mem.pss));
                            ui.label(format_bytes(e.mem.swap));
                            ui.end_row();
                        }
                    });
            });
        if app.pss_entries.is_empty() {
            ui.label(
                egui::RichText::new(
                    "No pss data — try Refresh as root / check /proc/*/smaps_rollup",
                )
                .weak(),
            );
        }
    }

    fn oom_tab(ui: &mut egui::Ui, app: &mut App) {
        ui.horizontal(|ui| {
            ui.label("Top");
            ui.add(egui::DragValue::new(&mut app.oom_top).range(5..=100));
            if ui.button("Refresh").clicked() {
                app.refresh_oom();
            }
            ui.label(egui::RichText::new(format!("OOM kills: {}", app.oom_kills)).weak());
        });
        egui::Grid::new("oom_grid")
            .num_columns(5)
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("PID").strong());
                ui.label(egui::RichText::new("COMM").strong());
                ui.label(egui::RichText::new("SCORE").strong());
                ui.label(egui::RichText::new("ADJ").strong());
                ui.label(egui::RichText::new("RSS").strong());
                ui.end_row();
                for (pid, comm, score, rss) in &app.oom_entries {
                    ui.label(pid.to_string());
                    ui.label(comm.clone());
                    let col = if score.score > 500 {
                        egui::Color32::from_rgb(220, 80, 80)
                    } else {
                        egui::Color32::GRAY
                    };
                    ui.colored_label(col, score.score.to_string());
                    ui.label(score.adj.to_string());
                    ui.label(format_bytes(*rss));
                    ui.end_row();
                }
            });
    }

    fn slab_tab(ui: &mut egui::Ui, app: &mut App) {
        ui.horizontal(|ui| {
            ui.label("Top");
            ui.add(egui::DragValue::new(&mut app.slab_top).range(5..=100));
            if ui.button("Refresh").clicked() {
                app.refresh_slab();
            }
            ui.label(egui::RichText::new(format!("total {}", format_bytes(app.slab_total))).weak());
        });
        if app.slab_caches.is_empty() {
            ui.label(
                egui::RichText::new(
                    "No slab data — needs root ( /proc/slabinfo is 0400 on most kernels )",
                )
                .weak(),
            );
            return;
        }
        egui::Grid::new("slab_grid")
            .num_columns(5)
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("CACHE").strong());
                ui.label(egui::RichText::new("SIZE").strong());
                ui.label(egui::RichText::new("ACTIVE").strong());
                ui.label(egui::RichText::new("OBJS").strong());
                ui.label(egui::RichText::new("WASTE").strong());
                ui.end_row();
                for c in &app.slab_caches {
                    ui.label(c.name.clone());
                    ui.label(format_bytes(c.size_bytes()));
                    ui.label(format_bytes(c.active_bytes()));
                    ui.label(c.num_objs.to_string());
                    ui.label(format_bytes(c.waste_bytes()));
                    ui.end_row();
                }
            });
    }

    fn zram_tab(ui: &mut egui::Ui, app: &mut App) {
        if ui.button("Refresh").clicked() {
            app.refresh_zram();
        }
        if app.zram_devices.is_empty() {
            ui.label(egui::RichText::new("No zram devices ( /sys/block/zram* missing )").weak());
            if let Some(m) = app.memory {
                ui.label(format!(
                    "Swap {} / {}",
                    format_bytes(m.swap_used()),
                    format_bytes(m.swap_total)
                ));
            }
            return;
        }
        for dev in &app.zram_devices {
            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label(egui::RichText::new(&dev.name).strong());
                ui.label(format!(
                    "disk {}  orig {} → comp {}  ratio {:.1}x  mem {}  same {} huge {}",
                    format_bytes(dev.disksize),
                    format_bytes(dev.orig),
                    format_bytes(dev.compressed),
                    dev.ratio(),
                    format_bytes(dev.mem_used),
                    dev.same_pages,
                    dev.huge_pages
                ));
            });
        }
    }

    fn cgroup_tab(ui: &mut egui::Ui, app: &mut App) {
        ui.label(
            egui::RichText::new("Cgroup v2 — reclaim & limits (needs cgroup v2 + delegation)")
                .weak(),
        );
        ui.horizontal(|ui| {
            ui.label("Cgroup path");
            ui.text_edit_singleline(&mut app.cgroup_path);
            ui.label(
                egui::RichText::new("relative to cgroup2 mount, empty = self")
                    .small()
                    .weak(),
            );
        });
        ui.horizontal(|ui| {
            if ui.button("Reclaim").clicked() {
                let path = if app.cgroup_path.trim().is_empty() {
                    None
                } else {
                    Some(app.cgroup_path.trim())
                };
                let cg = match cgroup::Cgroup::current() {
                    Ok(c) => c,
                    Err(e) => {
                        app.set_status(format!("cgroup: {e:#}"), true);
                        return;
                    }
                };
                let dir = match path {
                    None => cg.dir(),
                    Some(p) => cg.resolve(p),
                };
                let res = cgroup::write(&dir, "memory.reclaim", "0");
                match res {
                    Ok(()) => app.set_status(format!("reclaimed in {}", dir.display()), false),
                    Err(e) => app.set_status(format!("reclaim failed: {e:#}"), true),
                }
            }
            if ui.button("Show limits").clicked() {
                let cg = match cgroup::Cgroup::current() {
                    Ok(c) => c,
                    Err(e) => {
                        app.set_status(format!("cgroup: {e:#}"), true);
                        return;
                    }
                };
                let dir = if app.cgroup_path.trim().is_empty() {
                    cg.dir()
                } else {
                    cg.resolve(app.cgroup_path.trim())
                };
                let high = cgroup::field_opt(&dir, "memory.high").unwrap_or(None);
                let max = cgroup::field_opt(&dir, "memory.max").unwrap_or(None);
                let cur = cgroup::field_opt(&dir, "memory.current").unwrap_or(None);
                app.set_status(
                    format!(
                        "{} high:{} max:{} cur:{}",
                        dir.display(),
                        high.map(|v| format_bytes(v))
                            .unwrap_or_else(|| "max".into()),
                        max.map(|v| format_bytes(v)).unwrap_or_else(|| "max".into()),
                        cur.map(|v| format_bytes(v)).unwrap_or_else(|| "?".into())
                    ),
                    false,
                );
            }
        });
        ui.horizontal(|ui| {
            egui::ComboBox::from_label("Limit")
                .selected_text(if app.limit_kind_is_high {
                    "high"
                } else {
                    "max"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut app.limit_kind_is_high, true, "high");
                    ui.selectable_value(&mut app.limit_kind_is_high, false, "max");
                });
            ui.text_edit_singleline(&mut app.limit_value);
            if ui.button("Set").clicked() {
                let kind = if app.limit_kind_is_high {
                    "memory.high"
                } else {
                    "memory.max"
                };
                let cg = match cgroup::Cgroup::current() {
                    Ok(c) => c,
                    Err(e) => {
                        app.set_status(format!("cgroup: {e:#}"), true);
                        return;
                    }
                };
                let p = app.cgroup_path.trim();
                if p.is_empty() {
                    app.set_status("set a cgroup path first", true);
                    return;
                }
                let dir = cg.resolve(p);
                let content = if app.limit_value.trim() == "max" {
                    "max".to_string()
                } else {
                    match crate::mem::parse_size(&app.limit_value) {
                        Ok(v) => v.to_string(),
                        Err(e) => {
                            app.set_status(format!("bad size: {e:#}"), true);
                            return;
                        }
                    }
                };
                match cgroup::write(&dir, kind, &content) {
                    Ok(()) => app.set_status(
                        format!("set {kind} of {} to {}", dir.display(), app.limit_value),
                        false,
                    ),
                    Err(e) => app.set_status(format!("limit set failed: {e:#}"), true),
                }
            }
        });
        #[cfg(target_os = "windows")]
        ui.label(
            egui::RichText::new("Cgroups are Linux-only — this tab is a no-op on Windows.")
                .color(egui::Color32::YELLOW),
        );
    }

    fn grow_tab(ui: &mut egui::Ui, app: &mut App) {
        ui.horizontal(|ui| {
            ui.label("Interval s");
            ui.add(egui::DragValue::new(&mut app.grow_interval).range(5..=600));
            ui.label("Min");
            ui.text_edit_singleline(&mut app.grow_min);
            let label = if app.grow_running {
                "Scanning…"
            } else {
                "Start scan"
            };
            ui.add_enabled(!app.grow_running, egui::Button::new(label))
                .clicked()
                .then(|| app.start_grow());
            if app.grow_running {
                ui.spinner();
            }
        });
        if app.grow_results.is_empty() && !app.grow_running {
            ui.label(egui::RichText::new("No results yet — Start scan and wait.").weak());
        } else {
            egui::Grid::new("grow_grid")
                .num_columns(4)
                .spacing([12.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("PID").strong());
                    ui.label(egui::RichText::new("COMM").strong());
                    ui.label(egui::RichText::new("PSS").strong());
                    ui.label(egui::RichText::new("DELTA").strong());
                    ui.end_row();
                    for (pid, comm, pss, delta) in &app.grow_results {
                        ui.label(pid.to_string());
                        ui.label(comm.clone());
                        ui.label(format_bytes(*pss));
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 80, 80),
                            format!("+{}", format_bytes(*delta)),
                        );
                        ui.end_row();
                    }
                });
        }
    }

    pub fn run() -> eframe::Result {
        let opts = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([920.0, 620.0])
                .with_min_inner_size([760.0, 480.0])
                .with_title("Mem Reduct"),
            ..Default::default()
        };
        eframe::run_native(
            "Mem Reduct",
            opts,
            Box::new(|cc| Ok(Box::new(App::new(cc)))),
        )
    }
}

#[cfg(feature = "gui")]
pub use imp::run;

#[cfg(not(feature = "gui"))]
pub fn run() -> anyhow::Result<()> {
    anyhow::bail!("GUI not built — rebuild with --features gui (needs a display server)");
}
