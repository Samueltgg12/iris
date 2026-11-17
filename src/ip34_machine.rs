//! The Fuel (IP34 board, IP35 software platform): an R14000 on Bedrock, XBridge and the base I/O,
//! selected with `[machine] profile = "fuel_ip34"`; `[fuel]` selects the console and flash. The
//! IOC3 carries PS/2 (default) and the OHCI controller can take a USB pair. `[graphics] board`
//! "v12" or "v10" puts a VPro at crossbar widget 13. The node's approximations are in `ip35::node`.
//!
//! # What is approximated here
//!
//! The graphics flow-control registers store what IRIX writes but count no stores, so the
//! credit-starvation interrupt never fires. The board's DMA reaches memory through Bedrock's
//! inbound path as widget 13.
//!
//! Board composition ported from hw/mips/sgi/sn1-node.c in jamesbraid/qemu-sgi-origami.

#![warn(dead_code)]

use crate::config::{GraphicsBoard, MachineConfig};
use crate::cpu::parts::{self, Part};
use crate::dev::bedrock::{Bedrock, Population};
use crate::dev::bedrock_flash::{FirmwareRecord, Flash};
use crate::dev::heart::Clock;
use crate::dev::ioc3::{IP34, IP34_NO_PS2};
use crate::dev::isp12160::Isp12160;
use crate::dev::l1::{L1Config, L1};
use crate::dev::odyssey::Odyssey;
use crate::dev::timekeeper::Timekeeper;
use crate::dev::xbridge::{XBridge, HEAD_F};
use crate::hptimer::TimerManager;
use crate::ip35::{attach_ioc3, attach_usb, attach_vpro, xbridge_on};
use crate::ip35::node::NodeSpec;
use crate::ip35::serial::connect_serial_ports;
use crate::sn::BoardIo;
use crate::pci::DevFn;
use crate::sn::NodeBoard;
use crate::traits::Resettable;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

/// The Fuel's one node: its L1 answers with the configured station address,
/// and a configured revision or brick serial replaces the stock one.
pub(crate) fn node_specs(cfg: &MachineConfig) -> Result<Vec<NodeSpec>, String> {
    let mut l1 = L1Config { mac: cfg.network().mac, ..Default::default() };
    if let Some(revision) = cfg.fuel.l1_revision.as_deref().map(crate::dev::l1::parse_revision).transpose()? {
        l1.revision = Some(revision);
    }
    if let Some(serial) = &cfg.fuel.brick_serial {
        l1.brick_serial = Some(serial.clone());
    }
    Ok(vec![NodeSpec {
        name: "Fuel",
        module: &parts::R14000_500_SYSCLK_200,
        firmware: FirmwareRecord::FUEL,
        board_id: FUEL_BOARD_ID,
        banks_mib: FUEL_BANKS_MIB.to_vec(),
        processors: Population::SINGLE,
        absent: "",
        l1,
        flash: cfg.fuel.flash.as_deref().map(PathBuf::from),
    }])
}

<<<<<<< HEAD
impl FuelMachine {
    pub fn new(cfg: &MachineConfig) -> Result<Self, String> {
        cfg.validate()?;
        let prom = prom_image::resolve(cfg.machine.profile, &cfg.prom, 8 * 1024 * 1024)
            .map_err(|e| format!("Fuel {e}"))?
            .bytes;
        let timekeeper = Arc::new(Timekeeper::with_nvram_file(
            Path::new(&cfg.nvram),
        )?);
        let board = Arc::new(FuelBoard::with_config(FuelBoardConfig {
            payload: &prom,
            clock: host_clock(),
            flash_path: cfg.fuel.flash.as_deref().map(Path::new),
            station_mac: cfg.network().mac,
            timekeeper: timekeeper.clone(),
            ps2_input: cfg.fuel.ps2_input,
            usb_input: cfg.fuel.usb_input,
            l1: L1Identity {
                revision: cfg.fuel.l1_revision.as_deref().map(crate::dev::l1::parse_revision).transpose()?,
                brick_serial: cfg.fuel.brick_serial.clone(),
            },
            graphics: cfg.graphics_board(),
            graphics_pool: (cfg.graphics.threads, cfg.graphics.tile),
            graphics_raster: (cfg.graphics.raster, cfg.graphics.gpu_backend.as_str()),
        })?);
        let timers = Arc::new(TimerManager::new());
        let part = cfg.cpu();
        let mut exec = board.build_cpu(part);
        // The CPU's Compare matches blink the graphics status bar's LED.
        exec.core.fasttick_count = board.fasttick.clone();
        exec.core.set_timer_manager(timers.clone());
        exec.core.set_count_hz(
            cfg.clock
                .fixed_mhz
                .map_or(part.count_hz(), |mhz| (mhz * 1_000_000.0) as u64)
                .max(1),
        );
        exec.core.count_write_samples = true;
        let poll_board = board.clone();
        exec.batch_hook = Some(Box::new(move |_| poll_board.poll()));
        let cpu: Arc<dyn CpuDevice> = Arc::new(MipsCpu::<_, R12000ShadowCache>::new(*exec));
        board.status.set_cpu_cycles(cpu.cycles_ptr());
        board.status.set_count_hz(cpu.count_hz_atomic());
        connect_cpus("Fuel", &board.bedrock, &board.l1, &board.events, std::slice::from_ref(&cpu))?;
        board.bedrock.set_timer_manager(&timers);
        // After the windows above: each compile worker takes the cache's
        // geometry and the CPU model's policies once, as it starts, and
        // without the windows the geometry has no inline load or store path.
        // Workers read plain physical code addresses through the CPU's code
        // bus: the cached bus, plus the reset PROM's uncached-fetch window.
        // The uncached views would decode plain addresses as Hub registers.
        #[cfg(feature = "jitv2")]
        {
            let cpu_device: Arc<dyn Device> = cpu.clone().as_device();
            let jit = cpu.jitv2();
            let mut g = jit.lock();
            let (jitv2_threads, jitv2_threads_reason) = cfg.jitv2_pool_size();
            g.compile_queue.set_thread_count_with_reason(jitv2_threads.max(1), &jitv2_threads_reason);
            g.compile_queue.set_cpu(Arc::downgrade(&cpu_device));
            g.compile_queue.set_owner(Arc::downgrade(&jit));
            // Not under lockstep, as on the other machines: lockstep compiles
            // inline on the CPU thread, and a running pool both owns the
            // Codegen inline needs and deadlocks the CPU thread's flush.
            #[cfg(not(feature = "jitv2_lockstep"))]
            {
                let stats = g.stats.clone();
                g.compile_queue.start(board.code_bus(), stats);
            }
            // The compile pool keeps only a weak handle on the CPU. Forgetting
            // this strong one would keep the CPU, and through its batch hook
            // the board, its disks and its flash, alive after the machine.
            drop(cpu_device);
        }
        let scratch = Self::attach_scsi(cfg, &board)?;
        let console = match cfg.fuel.console {
            FuelConsole::L1 => 0,
            FuelConsole::Ioc3A => 1,
            FuelConsole::Ioc3B => 2,
        };
        let ci_serial = cfg.ci.then(|| board.serial[console].clone());
        Self::connect_serial(cfg, &board, console)?;
        let monitor = Monitor::new();
        monitor.register_device(init_devlog());
        monitor.register_device(Arc::new(crate::cpu::cpu_monitor::CpuMonitor::new(vec![cpu.clone()])));
        monitor.register_device(PerfMonitor::new(
            cpu.running_flag(), cpu.cycles_ptr(), cpu.fasttick_count(), None, None,
        ));
        for device in board.monitor_devices() {
            monitor.register_device(device);
        }
        cpu.register_locks();
        board.scsi.register_locks();
        board.ioc3.register_locks();
        board.xbridge.register_locks();
        board.flash.register_locks();
        board.bedrock.register_locks();
        let (mut nat, mut frames) = (None, None);
        let net = GatewayConfig::from_network(&cfg.network());
        if let Some(link) = Link::with_gateway(net.clone())? {
            frames = Some(link.frames());
            nat = Some((link.nat(), (net.client_ip, net.gateway_ip, net.netmask)));
            board.eth.attach(link);
        }
        let events = board.receiver.lock().take();
        eprintln!(
            "iris: Fuel IP34, {}, 512 MiB, {}, serial console {:?}",
            cfg.cpu().label(),
            if board.vpro.is_some() { cfg.graphics_board().label() } else { "no graphics" },
            cfg.fuel.console
        );
        Ok(Self {
            board,
            cpu,
            timers,
            monitor: Arc::new(monitor),
            monitor_port: cfg.monitor_port.unwrap_or(DEFAULT_MONITOR_PORT),
            ci_serial,
            console,
            ci: cfg.ci,
            scratch,
            timekeeper,
            nvram: cfg.nvram.clone(),
            nat,
            frames,
            events,
            slot: None,
            lifecycle: GuestState::Idle,
        })
    }

    fn attach_scsi(cfg: &MachineConfig, board: &FuelBoard) -> Result<Option<PathBuf>, String> {
        let attached = attach_from_config(cfg, Some("fuel"), &|channel| {
            board.scsi.bus_for_channel(channel)
                .ok_or_else(|| format!("controller {channel} (a Fuel has channels 0 and 1)"))
        })?;
        if let Some(device) = attached.daynaports.first() {
            return Err(format!("{}: kind = \"daynaport\" is not available on the Fuel",
                scsi_label(device.controller, device.id)));
        }
        Ok(attached.scratch)
    }

    pub(crate) fn connect_serial(cfg: &MachineConfig, board: &FuelBoard, console: usize) -> Result<(), String> {
        let [a, b] = cfg.serial_ports();
        let ioc3 = |port: usize| -> Box<dyn Fn(Arc<dyn SerialBackend>) + '_> {
            Box::new(move |backend| board.ioc3.set_serial_backend(port, backend))
        };
        let ports = [
            SerialPort {
                label: "L1",
                port: cfg.fuel.l1_port.unwrap_or(8882),
                setting: "fuel.l1_port",
                ci: &board.serial[0],
                log: cfg.fuel.l1_log.clone(),
                connect: Box::new(|backend| board.l1.set_serial_backend(backend)),
            },
            SerialPort { label: "IOC3 A", port: a, setting: "serial_port_a", ci: &board.serial[1], log: None, connect: ioc3(0) },
            SerialPort { label: "IOC3 B", port: b, setting: "serial_port_b", ci: &board.serial[2], log: None, connect: ioc3(1) },
        ];
        connect_serial_ports(cfg, "Fuel", &ports, console)
    }
}

impl Workstation for FuelMachine {
    fn start(&mut self) {
        self.lifecycle = GuestState::Starting;
        self.monitor
            .clone()
            .start_server(format!("127.0.0.1:{}", self.monitor_port));
        self.restart_peripherals();
        if !self.ci {
            self.cpu_start();
        }
    }
    fn stop(&mut self) {
        self.cpu.clone().as_device().stop();
        self.board.stop();
        if let Err(e) = self.timekeeper.flush_nvram() {
            dlog!(LogModule::Bedrock, "Fuel NVRAM: {e}");
        }
        if self.lifecycle != GuestState::Idle {
            self.lifecycle = GuestState::Halted;
        }
    }
    fn reset(&mut self) {
        self.stop();
        self.cpu.power_on();
        self.board.bedrock.power_on();
        self.board.l1.power_on();
        self.board.flash.power_on();
        self.board.xbridge.power_on();
        self.board.ioc3.power_on();
        self.board.scsi.power_on();
        self.board.usb.power_on();
        if let Some(vpro) = &self.board.vpro {
=======
/// The Fuel's I/O around its node: the XBridge, the VPro in the graphics slot, and the base I/O.
pub(crate) fn build_io(cfg: &MachineConfig, nodes: &[NodeBoard<Bedrock, Flash>], l1s: &[Arc<L1>], _: &Arc<TimerManager>, clock: &Clock) -> Result<BoardIo, String> {
    let bedrock = &nodes[0].chip;
    let xbridge = xbridge_on(bedrock);
    // The status bar's activity bits, shared with the base I/O's devices.
    let heartbeat = Arc::new(AtomicU64::new(0));
    let fasttick = Arc::new(AtomicU64::new(0));
    let status = crate::disp::StatusSource::new(heartbeat.clone(), fasttick.clone());
    let graphics = cfg.graphics_board();
    let vpro = (graphics != GraphicsBoard::None)
        .then(|| attach_vpro(cfg, bedrock, WIDGET_VPRO, bedrock.interrupt_sender(WIDGET_VPRO, None), &status, |vpro| Ok(xbridge.attach_widget(WIDGET_VPRO, vpro))))
        .transpose()?;
    eprintln!("iris: {}", if vpro.is_some() { graphics.label() } else { "no graphics" });
    let mut io = build_baseio(cfg, &l1s[0], &xbridge, clock, heartbeat, vpro.clone())?;
    io.status = Some(status);
    io.fasttick = Some(fasttick);
    if let Some(vpro) = vpro {
        io.display = Some(vpro.display());
        io.monitor.push(vpro.clone());
        io.devices.push(vpro.clone());
        let base_reset = std::mem::replace(&mut io.reset, Box::new(|| {}));
        io.reset = Box::new(move || {
            base_reset();
>>>>>>> d4fea4a20c85d08504aa03046b62cdde0e13f62c
            vpro.power_on();
        });
    }
    Ok(io)
}

/// The Fuel's base I/O on its XBridge's head F: the ISP12160, the IOC3 and the OHCI controller. A
/// link reset of head F resets them, and of the graphics port the `vpro` behind it.
fn build_baseio(
    cfg: &MachineConfig,
    l1: &Arc<L1>,
    xbridge: &Arc<XBridge>,
    clock: &Clock,
    heartbeat: Arc<AtomicU64>,
    vpro: Option<Arc<Odyssey>>,
) -> Result<BoardIo, String> {
    let timekeeper = Arc::new(Timekeeper::with_nvram_file(Path::new(&cfg.nvram))?);
    let pci = xbridge.bus(HEAD_F);
    let board = if cfg.fuel.ps2_input { &IP34 } else { &IP34_NO_PS2 };
    let (ioc3, serial) = attach_ioc3(cfg, pci, board, l1, &timekeeper, cfg.fuel.l1_port.unwrap_or(8882), "fuel.l1_port", cfg.fuel.l1_log.clone());
    let eth = ioc3.eth().expect("the Fuel's IOC3 has Ethernet").clone();
    let scsi = Arc::new(Isp12160::new(Isp12160::new_bus(), Isp12160::new_bus()));
    scsi.set_heartbeat(heartbeat.clone());
    ioc3.set_heartbeat(heartbeat);
    pci.attach(DevFn { device: 1, function: 0 }, scsi.clone());
    let usb = attach_usb(pci, &ioc3, clock, cfg.fuel.usb_input)?;
    let (reset_ioc3, reset_scsi, reset_usb) = (ioc3.clone(), scsi.clone(), usb.clone());
    let reset_vpro = vpro.as_ref().map(Arc::downgrade);
    xbridge.set_reset_output(Arc::new(move |port| {
        if port == HEAD_F {
            reset_ioc3.power_on();
            reset_scsi.power_on();
            reset_usb.power_on();
        }
        // A link reset of the graphics port resets the board behind it.
        if port == WIDGET_VPRO {
            if let Some(vpro) = reset_vpro.as_ref().and_then(|v| v.upgrade()) {
                vpro.power_on();
            }
        }
    }));
    let (table, product) = (cfg.machine.profile.table_name().unwrap_or_default(), cfg.machine.profile.product_name().unwrap_or_default());
    let scratch = crate::scsi::attach_without_daynaport(cfg, table, product, &|channel| {
        scsi.bus_for_channel(channel).ok_or_else(|| format!("controller {channel} (a {product} has channels 0 and 1)"))
    })?;
    let console_index = cfg.fuel.console.index();
    connect_serial_ports(cfg, "Fuel", &serial, console_index)?;
    let console = cfg.ci.then(|| serial[console_index].ci.clone());
    scsi.register_locks();
    ioc3.register_locks();
    xbridge.register_locks();
    let (reset_xbridge, reset_ioc3, reset_scsi, reset_usb, reset_clock) = (xbridge.clone(), ioc3.clone(), scsi.clone(), usb.clone(), timekeeper.clone());
    let poll_usb = usb.clone();
    let disks = scsi.clone();
    Ok(BoardIo {
        devices: vec![ioc3.clone(), scsi.clone(), usb],
        reset: Box::new(move || {
            reset_xbridge.power_on();
            reset_ioc3.power_on();
            reset_scsi.power_on();
            reset_usb.power_on();
            reset_clock.power_on();
        }),
        poll: Some(Arc::new(move || poll_usb.sync_clock())),
        at_batch: None,
        monitor: vec![scsi.clone(), ioc3.ps2()],
        ps2: ioc3.ps2(),
        flush_nvram: Box::new(move || timekeeper.flush_nvram()),
        console,
        attach_net: Box::new(move |link| eth.attach(link)),
        disks: Box::new(move |f| f(&(0..2).filter_map(|channel| disks.bus_for_channel(channel)).collect::<Vec<_>>())),
        disc_changer: Some(scsi),
        scratch,
        display: None,
        status: None,
        fasttick: None,
        cpus: None,
    })
}

/// The processors a Fuel can be built with, the default first: the R14000A
/// the Fuel ships with.
pub const CPUS: &[&Part] = &[&parts::R14000_500];

/// The Fuel's board-id word: class 4 (bits 15:12; OpenBSD names it IP35_FUEL), nibble 4 (the
/// PROM 6.210 speed-table row of the 500 MHz, 2 MB module; established). Bits 11:4 and 63:16
/// of a real word are unknown and read zero.
pub(crate) const FUEL_BOARD_ID: u64 = 0x4004;
/// Two 256 MiB banks, locational banks 0 and 1.
const FUEL_BANKS_MIB: [u32; 2] = [256, 256];

/// The crossbar widget of the Fuel's graphics slot: a Fuel IRIX inventory
/// lists its VPro under xtalk/13, and OpenBSD finds Odyssey at widget 13.
pub const WIDGET_VPRO: u8 = 13;

#[cfg(test)]
mod tests;
