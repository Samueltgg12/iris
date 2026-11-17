//! The Origin 3200 and the Onyx 3000: one or two C-bricks, each a Bedrock node with a 400 MHz R12000,
//! and an I-brick on the first C-brick's XIO connector carrying the base I/O. Two C-bricks are joined
//! by a direct NUMAlink with no router.
//!
//! Selected with `[machine] profile = "origin3200_ip35"` or `"onyx3000_ip35"`; `[origin3200]` selects
//! the C-brick count, console and flash, and `[onyx3000]` takes its keys except `c_bricks`. `banks` is
//! per C-brick (256 + 256 MiB by default). The first C-brick's L1 is the console.
//!
//! The I-brick is an XBridge whose heads E and F are widgets 14 and 15, its host port A (widget 10)
//! connected to the C-brick; head F's bus (the IO7) holds a Fibre Channel adapter at device 1 (always
//! attached, with the disks of `[fibre_channel]`, 64-bit data phases), the IOC3 at 4 and USB at 5. Port B
//! ([`SPARE_PORT`]) is declared and empty.
//!
//! The Onyx 3000's `[graphics] board` picks the graphics brick. Only ir3's widget 11 is established;
//! the rest is the model's composition. `board = "ir3"` is one C-brick, the I-brick and a single-pipe
//! G-brick (an Onyx 3200): the pipe sits at widget 11 of the I-brick's XBridge ([`SPARE_PORT`]), its
//! DMA and interrupts reaching the C-brick through the XBridge's host port. `board = "v12"` is two
//! C-bricks joined by NUMAlink, the first with the I-brick, the second with a V-brick (an XBridge and
//! the V12 at widget 8) on its own XIO connector; the V-brick's connection to a C-brick is
//! established, its having no PCI devices inferred.
//!
//! # What is approximated here
//!
//! - A C-brick holds two or four processors; the model runs one, so B, C and D are listed absent in
//!   the flash with a one-PIMM L1 record (the real count for one processor is not established).
//! - The low nibble of the board identification word is a choice: 2 is the 400 MHz, 8 MiB row.
//! - The I-brick's L1 is a stub: the first C-brick's answers its bay and letter and the rest as its own.
//! - The IEEE 1394 controller is not modelled, so the PROM prints `No 1394 Firewire found on BASEIO`.
//! - The IO7's IOC3 is the Fuel's with the placeholder subsystem id `0xffff`.
//! - The V12's widget: 8 and 12 are equal candidates from the X-brick's slot table; 8 is the choice
//!   ([`VPRO_WIDGET`]). The V-brick's XBridge has both PCI heads, empty, as the chip always does.
//! - The V-brick's controller answers as an X-brick ([`V_BRICK`]): the type letter, and a stub serving
//!   every other request as its C-brick's; its real board records are not established.
//! - The G-brick's L1 and its link are not modelled; the pipe is a widget of the I-brick's XBridge
//!   (established: each pipe sits at widget 11 of an I-brick or X-brick XBridge over a direct link;
//!   one behind an N-brick is not modelled).
//! - The I-brick's controller lists the G-brick at port 2 of its attached-brick reply, letter `G`, at
//!   [`G_BRICK_BAY`] (the model's choice; the PROM needs a name there or takes the port for a second host).
//! - The status bar's activity bits and clock blink are not wired: the pipe's status has its own counters.
//! - Which products an Onyx 3000 takes follows `KonaHost::Onyx3000`: InfiniteReality3 only on a G-brick.
//! - The I-brick has no PS/2 path (observed in the PROM run), so without a USB keyboard in its OHCI the
//!   PROM prints "Cannot connect to keyboard" and the board draws nothing; with one, `console = g` works.
//!
//! Board composition ported from hw/mips/sgi/sn-machine.c and
//! hw/mips/sgi/sn1-node.c in jamesbraid/qemu-sgi-origami, where no Onyx 3000 hardware graph has been captured.

use crate::config::{MachineConfig, MachineProfile};
use crate::cpu::parts::{self, Part};
use crate::dev::bedrock::{Bedrock, Population};
use crate::dev::bedrock_flash::{FirmwareRecord, Flash, Header};
use crate::dev::heart::Clock;
use crate::dev::ioc3::IO7;
use crate::dev::isp2200::card::Card;
use crate::dev::kona::{Kona, KonaConfig};
use crate::dev::l1::{AttachedBrick, BoardIdentity, ComputeNode, L1Config, L1};
use crate::dev::timekeeper::Timekeeper;
use crate::dev::xbridge::{XBridge, HEAD_F, HOST_WIDGET};
use crate::hptimer::TimerManager;
use crate::ip35::node::NodeSpec;
use crate::ip35::serial::connect_serial_ports;
use crate::ip35::{attach_ioc3, attach_usb, attach_vpro, xbridge_on};
use crate::sn::BoardIo;
use crate::sn::NodeBoard;
use crate::traits::{Device, Resettable};
use std::path::Path;
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

/// The processors an Origin 3200 C-brick is built with: the 400 MHz R12000
/// with an 8 MiB secondary cache.
pub const CPUS: &[&Part] = &[&parts::R12000_400_8M];

/// The C-brick's 400 MHz / 8 MiB PROM record selects these inputs; on a mismatch with the
/// processor's Config the PROM disables that processor slice, prints a notice and starts up. The
/// mode word is the stock IP35 PROM's row for that population (Config 0x5414a785); the machine type is 0,
/// the class the PROM derives from [`BOARD_ID`].
const FIRMWARE: FirmwareRecord = FirmwareRecord {
    cpu_mode_word: parts::R12000_400_8M_SYSCLK_200.mode_word,
    cpu_clock_hz: 400_000_000,
    machine_type: 0,
    header: Header::Refreshed,
};

/// Bits 15:12 of the board identification word are the product code (0 is a C-brick) and the low
/// nibble the PROM's configuration row; 2 is the 400 MHz, 8 MiB row (a choice, not established).
const BOARD_ID: u64 = 2;

/// The L1's brick type character for a C-brick.
const L1_TYPE: u32 = 0x43;

/// The I-brick of the Origin 3200 in module 001i14, as the first C-brick's
/// L1 reports it.
const I_BRICK: AttachedBrick = AttachedBrick { bay: 14, type_code: 0x49, graphics_bay: None };

/// A C-brick's node board and processor module as its L1 names them: the names are the product's,
/// the part and revision the marked placeholders the L1 model uses. None is observed.
const NODE_BOARD: BoardIdentity = BoardIdentity { name: "NODE", part: "NOTOBSERVED0", revision: "00" };
const PIMM: BoardIdentity = BoardIdentity { name: "PIMM", part: "NOTOBSERVED0", revision: "00" };

/// Where each C-brick sits: the rack bay its L1 reports (module 001c04 is bay
/// 4) and the synthetic serial of its L1.
const BAYS: [(u32, &str); 2] = [(4, "EMUN00"), (7, "EMUN01")];

/// The second Crosstown2 port of the I-brick, declared with nothing connected to it.
const SPARE_PORT: u8 = 11;

/// The C-bricks of a machine: `count`, the first with `first_attached` (the I-brick) on its XIO
/// connector and the second, if any, with `second_attached`.
fn c_bricks(
    cfg: &MachineConfig,
    count: usize,
    first_attached: AttachedBrick,
    second_attached: Option<AttachedBrick>,
) -> Result<Vec<NodeSpec>, String> {
    let section = cfg.ip35_section();
    let banks = cfg.ip35_banks()?;
    let revision = section.l1_revision.as_deref().map(crate::dev::l1::parse_revision).transpose()?;
    (0..count)
        .map(|index| {
            let (bay, serial) = BAYS[index];
            let mut mac = cfg.network().mac;
            mac[5] = mac[5].wrapping_add(index as u8);
            let mut l1 = L1Config {
                bay,
                type_code: L1_TYPE,
                mac,
                brick_serial: Some(serial.into()),
                node_board: Some(NODE_BOARD),
                attached: if index == 0 { Some(first_attached) } else { second_attached },
                compute_node: Some(ComputeNode { index: index as u8, pimm_mask: 1, pimm: PIMM }),
                ..Default::default()
            };
            if index == 0 {
                if let Some(revision) = revision {
                    l1.revision = Some(revision);
                }
                if let Some(serial) = &section.brick_serial {
                    l1.brick_serial = Some(serial.clone());
                }
            }
<<<<<<< HEAD
            Some(windows)
        } else {
            None
        };
        let io = B::wire(&nodes.iter().collect::<Vec<_>>(), cfg)?;
        Ok(Self { nodes, processors, specs, io, windows, events: merged_rx, _board: std::marker::PhantomData })
    }
}

/// What a node's processor brings up to date every batch: the node's boot
/// flash clock, and on the first node the board's clocked devices.
struct NodePoll {
    clock: Clock,
    last: AtomicU64,
    running: AtomicBool,
    flash: Arc<crate::dev::bedrock_flash::Flash>,
    board: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl NodePoll {
    fn new(clock: Clock, flash: Arc<crate::dev::bedrock_flash::Flash>, board: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        Self { last: AtomicU64::new(clock()), clock, running: AtomicBool::new(false), flash, board }
    }

    fn start(&self) {
        self.last.store((self.clock)(), Ordering::Release);
        self.running.store(true, Ordering::Release);
    }

    fn stop(&self) {
        self.poll();
        self.running.store(false, Ordering::Release);
    }

    fn poll(&self) {
        if self.running.load(Ordering::Acquire) {
            let now = (self.clock)();
            let previous = self.last.swap(now, Ordering::AcqRel);
            self.flash.advance_ns(now.saturating_sub(previous));
            if let Some(board) = &self.board {
                board();
            }
        }
    }
}

/// A C-brick with its processor.
struct Unit {
    node: Node,
    cpu: Arc<dyn CpuDevice>,
    poll: Arc<NodePoll>,
}

pub(crate) struct Ip35Machine<B: Ip35Board> {
    units: Vec<Unit>,
    io: BoardIo,
    /// The nodes' windows, which the chips hold weakly.
    _windows: Option<Arc<Windows>>,
    _world: Arc<World>,
    timers: Arc<TimerManager>,
    monitor: Arc<Monitor>,
    monitor_port: u16,
    ci_serial: Option<Arc<CiSerialBackend>>,
    ci: bool,
    scratch: Option<PathBuf>,
    nvram: String,
    frames: Option<Arc<AtomicU64>>,
    nat: Option<(Arc<NatControl>, (Ipv4Addr, Ipv4Addr, Ipv4Addr))>,
    events: Option<Receiver<MachineEvent>>,
    slot: Option<MachineSlot>,
    lifecycle: GuestState,
    _board: std::marker::PhantomData<fn() -> B>,
}

impl<B: Ip35Board> Ip35Machine<B> {
    pub fn new(cfg: &MachineConfig) -> Result<Self, String> {
        cfg.validate()?;
        let prom = prom_image::resolve(cfg.machine.profile, &cfg.prom, 8 * 1024 * 1024)
            .map_err(|e| format!("{} {e}", B::NAME))?
            .bytes;
        let Chassis { nodes, processors, specs, io, windows, events, .. } = Chassis::<B>::build(cfg, &prom, host_clock())?;
        let count = nodes.len();
        let timers = Arc::new(TimerManager::new());
        let board_poll = io.poll.clone();
        let mut units = Vec::new();
        for (index, ((node, spec), bus)) in nodes.into_iter().zip(&specs).zip(processors).enumerate() {
            let poll = Arc::new(NodePoll::new(host_clock(), node.flash.clone(), (index == 0).then(|| board_poll.clone())));
            let count_hz = cfg.clock.count_hz(spec.part).max(1);
            let cpu = crate::with_cpu!(spec.part, true, false, C => Self::start_cpu::<C>(&node, spec, bus, index, count, &timers, count_hz, poll.clone()));
            units.push(Unit { node, cpu, poll });
        }
        // One body of park slots for every processor of every node: a bank
        // configuration write on either node holds the others still, the
        // other node's processors included, whichever node's thread makes it.
        let world = Arc::new(World::new());
        world.set_slots(units.iter().map(|u| u.cpu.park_slot()).collect());
        for u in &units {
            u.node.bedrock.set_world(world.clone());
            let cpus = std::slice::from_ref(&u.cpu);
            if count > 1 {
                connect_placed_cpus(B::NAME, &u.node.bedrock, &u.node.l1, &u.node.events, cpus)?;
            } else {
                connect_cpus(B::NAME, &u.node.bedrock, &u.node.l1, &u.node.events, cpus)?;
            }
            u.node.bedrock.set_timer_manager(&timers);
        }
        #[cfg(feature = "jitv2")]
        for u in &units {
            let cpu_device: Arc<dyn Device> = u.cpu.clone().as_device();
            let jit = u.cpu.jitv2();
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
                drop(g);
                crate::cpu::jitv2::jitv2::start_compiling(&*u.cpu, &jit, cfg.jitv2.compile, u.node.code.clone());
            }
            // The compile pool keeps only a weak handle on the CPU. Forgetting
            // this strong one would keep the CPU, and through its batch hook
            // the board, its disks and its flash, alive after the machine.
            drop(cpu_device);
        }
        let scratch = None;
        let ports: Vec<_> = io.serial.iter().map(|line| SerialPort {
            label: line.label,
            port: line.port,
            setting: line.setting,
            ci: &line.ci,
            log: line.log.clone(),
            connect: Box::new(|backend| (line.connect)(backend)),
        }).collect();
        connect_serial_ports(cfg, B::NAME, &ports, io.console)?;
        drop(ports);
        let ci_serial = cfg.ci.then(|| io.serial[io.console].ci.clone());
        let first = &units[0].cpu;
        if let Some(status) = &io.status {
            status.set_cpu_cycles(first.cycles_ptr());
            status.set_count_hz(first.count_hz_atomic());
        }
        let monitor = Monitor::new();
        monitor.register_device(init_devlog());
        monitor.register_device(Arc::new(crate::cpu::cpu_monitor::CpuMonitor::new(units.iter().map(|u| u.cpu.clone()).collect())));
        monitor.register_device(PerfMonitor::new(first.running_flag(), first.cycles_ptr(), first.fasttick_count(), None, None));
        for u in &units {
            monitor.register_device(u.node.bedrock.physical().clone());
        }
        monitor.register_device(units[0].node.l1.clone());
        for device in &io.monitor {
            monitor.register_device(device.clone());
        }
        // `scsi2` and `cow2`: the Fibre Channel loop's disks.
        if let Some(fibre) = &io.fibre {
            monitor.register_device(fibre.clone());
        }
        for u in &units {
            u.cpu.register_locks();
            u.node.flash.register_locks();
            u.node.bedrock.register_locks();
        }
        (io.register_locks)();
        if let Some(fibre) = &io.fibre {
            fibre.register_locks();
        }
        let (mut nat, mut frames) = (None, None);
        let net = GatewayConfig::from_network(&cfg.network());
        if let Some(link) = Link::with_gateway(net.clone())? {
            frames = Some(link.frames());
            nat = Some((link.nat(), (net.client_ip, net.gateway_ip, net.netmask)));
            io.eth.attach(link);
        }
        eprintln!(
            "iris: {}, {count} C-brick{} of {} MiB, {}, serial console {}",
            B::NAME,
            if count == 1 { "" } else { "s" },
            specs[0].banks_mib.iter().sum::<u32>(),
            specs[0].part.label(),
            io.serial[io.console].label,
        );
        Ok(Self {
            units,
            io,
            _windows: windows,
            _world: world,
            timers,
            monitor: Arc::new(monitor),
            monitor_port: cfg.monitor_port.unwrap_or(DEFAULT_MONITOR_PORT),
            ci_serial,
            ci: cfg.ci,
            scratch,
            nvram: cfg.nvram.clone(),
            nat,
            frames,
            events: Some(events),
            slot: None,
            lifecycle: GuestState::Idle,
            _board: std::marker::PhantomData,
=======
            Ok(NodeSpec {
                name: "C-brick",
                module: &parts::R12000_400_8M_SYSCLK_200,
                firmware: FIRMWARE,
                board_id: BOARD_ID,
                banks_mib: banks.clone(),
                processors: Population::SINGLE,
                // One processor runs: B, C and D hold none.
                absent: "BCD",
                l1,
                flash: crate::sn::per_node(&section.flash, index),
            })
>>>>>>> d4fea4a20c85d08504aa03046b62cdde0e13f62c
        })
        .collect()
}


/// The I-brick's base I/O on `xbridge` (the first C-brick's crossbar): a Fibre Channel adapter at
/// device 1 of head F, the IOC3 and USB. Head F's link reset resets the IOC3 and USB, and any
/// other port's goes to `link_reset`, for a product with a board at the I-brick's other port.
fn build_baseio(
    cfg: &MachineConfig,
    l1s: &[Arc<L1>],
    xbridge: &Arc<XBridge>,
    clock: &Clock,
    link_reset: impl Fn(u8) + Send + Sync + 'static,
) -> Result<BoardIo, String> {
    let l1 = &l1s[0];
    let timekeeper = Arc::new(Timekeeper::with_nvram_file(Path::new(&cfg.nvram))?);
    // The I-brick's head F bus, from published `hinv -mv` of an Origin 3200, an Onyx 3000 and an
    // Origin 3400: QLogic 2200 at slot 1, IOC3 at 4, USB at 5 (1394 at 6 unmodelled). Head F as
    // widget 15 and E as 14 is inferred from the port letters; head E's bus is empty.
    let pci = xbridge.bus(HEAD_F);
    // The on-board Fibre Channel adapter, with the disks of `[fibre_channel]`.
    let card = Card::open(&cfg.fibre_channel.clone().unwrap_or_default(), cfg.ci)?;
    card.attach(pci, 1);
    // The slot runs 64-bit data phases, inferred from the PROM: its rings are right only if the
    // bridge reverses each adapter DMA doubleword as it does on such a slot.
    xbridge.set_dma_64bit(HEAD_F, 1, true);
    eprintln!("iris: Fibre Channel on the I-brick, {} loop disk(s)", cfg.fibre_channel.as_ref().map_or(0, |f| f.disks.len()));
    let fibre = card.fc;
    let section = cfg.ip35_section();
    let (ioc3, serial) = attach_ioc3(
        cfg,
        pci,
        &IO7,
        l1,
        &timekeeper,
        section.l1_port.unwrap_or(8882),
        if cfg.machine.profile == MachineProfile::Onyx3000Ip35 { "onyx3000.l1_port" } else { "origin3200.l1_port" },
        section.l1_log.clone(),
    );
    let eth = ioc3.eth().expect("the IO7's IOC3 has Ethernet").clone();
    // The I-brick's keyboard and mouse are USB, not the IOC3's PS/2 ports.
    let usb = attach_usb(pci, &ioc3, clock, true)?;
    let (reset_ioc3, reset_usb) = (ioc3.clone(), usb.clone());
    xbridge.set_reset_output(Arc::new(move |port| {
        if port == HEAD_F {
            reset_ioc3.power_on();
            reset_usb.power_on();
        }
        link_reset(port);
    }));
    xbridge.register_locks();
    ioc3.register_locks();
    fibre.register_locks();
    let console_index = section.console.index();
    connect_serial_ports(cfg, cfg.machine.profile.product_name().unwrap_or_default(), &serial, console_index)?;
    let console = cfg.ci.then(|| serial[console_index].ci.clone());
    let (reset_xbridge, reset_ioc3, reset_usb, reset_clock) = (xbridge.clone(), ioc3.clone(), usb.clone(), timekeeper.clone());
    let poll_usb = usb.clone();
    // `scsi2` and `cow2`: the Fibre Channel loop's disks.
    let monitor: Vec<Arc<dyn Device>> = vec![ioc3.ps2(), fibre.clone()];
    let scsi_fibre = fibre.clone();
    Ok(BoardIo {
        devices: vec![ioc3.clone(), usb, fibre],
        reset: Box::new(move || {
            reset_xbridge.power_on();
            reset_ioc3.power_on();
            reset_usb.power_on();
            reset_clock.power_on();
        }),
        poll: Some(Arc::new(move || poll_usb.sync_clock())),
        at_batch: None,
        monitor,
        ps2: ioc3.ps2(),
        flush_nvram: Box::new(move || timekeeper.flush_nvram()),
        console,
        attach_net: Box::new(move |link| eth.attach(link)),
        disks: Box::new(move |f| {
            use crate::scsi_monitor::ScsiTargets;
            f(&[scsi_fibre.scsi_bus()])
        }),
        disc_changer: None,
        scratch: None,
        display: None,
        status: None,
        fasttick: None,
        cpus: None,
    })
}

/// Where an InfiniteReality pipe sits: widget 11 of the I-brick's XBridge,
/// its port B.
const PIPE_WIDGET: u8 = SPARE_PORT;

/// The rack bay the I-brick's controller reports for the G-brick: a choice, as no inventory places
/// one (the C-brick is at bay 4, the I-brick at 14).
const G_BRICK_BAY: u32 = 24;

/// The V-brick's V12 at widget 8 of its XBridge. Approximation: no V-brick widget listing is
/// captured; 8 and 12 are equal candidates from the X-brick's slot table.
const VPRO_WIDGET: u8 = 8;

/// The V-brick as its C-brick's controller reports it. Approximation: the L1 firmware names none;
/// it runs the X-brick's personality, so the type letter is `X`, the bay a choice and the rest the
/// C-brick's.
const V_BRICK: AttachedBrick = AttachedBrick { bay: 11, type_code: b'X' as u32, graphics_bay: None };

/// The C-bricks the profile names: the Origin 3200's one or two, the Onyx 3000's one with a G-brick
/// or two with a V-brick.
pub(crate) fn node_specs(cfg: &MachineConfig) -> Result<Vec<NodeSpec>, String> {
    if cfg.machine.profile != MachineProfile::Onyx3000Ip35 {
        c_bricks(cfg, usize::from(cfg.origin3200.c_bricks), I_BRICK, None)
    } else if cfg.graphics_board().is_vpro() {
        c_bricks(cfg, 2, I_BRICK, Some(V_BRICK))
    } else {
        c_bricks(cfg, 1, AttachedBrick { graphics_bay: Some(G_BRICK_BAY), ..I_BRICK }, None)
    }
}

/// The I-brick on the first C-brick, and the Onyx 3000's graphics brick behind it.
pub(crate) fn build_io(cfg: &MachineConfig, nodes: &[NodeBoard<Bedrock, Flash>], l1s: &[Arc<L1>], _: &Arc<TimerManager>, clock: &Clock) -> Result<BoardIo, String> {
    let xbridge = xbridge_on(&nodes[0].chip);
    if cfg.machine.profile != MachineProfile::Onyx3000Ip35 {
        return build_baseio(cfg, l1s, &xbridge, clock, |_| {});
    }
    if cfg.graphics_board().is_vpro() {
        let mut io = build_baseio(cfg, l1s, &xbridge, clock, |_| {})?;
        let v_node = nodes.get(1).ok_or("the V-brick needs a C-brick of its own")?;
        attach_vbrick(v_node, cfg, &mut io)?;
        return Ok(io);
    }
    let (product, rm_count) = cfg
        .graphics_board()
        .kona()
        .ok_or_else(|| format!("graphics.board \"{}\" is not a board of the Onyx 3000", cfg.graphics_board().as_str()))?;
    let status = crate::disp::StatusSource::new(Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let pipe = attach_ir3(&nodes[0].chip, &xbridge, cfg, KonaConfig::new(product, rm_count)?, &status)?;
    eprintln!("iris: {} on the G-brick at I-brick widget {PIPE_WIDGET}", product.spec().label);
    // A link reset of the pipe's port resets the widget's Crosstalk registers, not the board.
    let weak = Arc::downgrade(&pipe);
    let mut io = build_baseio(cfg, l1s, &xbridge, clock, move |port| {
        if port == PIPE_WIDGET {
            if let Some(pipe) = weak.upgrade() {
                pipe.link_reset();
            }
        }
    })?;
    io.display = Some(pipe.display());
    io.status = Some(status);
    io.monitor.push(pipe.clone());
    io.devices.push(pipe.clone());
    let base_reset = std::mem::replace(&mut io.reset, Box::new(|| {}));
    io.reset = Box::new(move || {
        base_reset();
        pipe.power_on();
    });
    Ok(io)
}

/// A V-brick on `node`'s XIO connector: an XBridge with no PCI devices and the V12 at
/// [`VPRO_WIDGET`], its DMA and interrupts going through the node's Bedrock as any widget's do.
fn attach_vbrick(node: &NodeBoard<Bedrock, Flash>, cfg: &MachineConfig, io: &mut BoardIo) -> Result<(), String> {
    let bedrock = &node.chip;
    let xbridge = xbridge_on(bedrock);
    let status = crate::disp::StatusSource::new(Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
    let vpro = attach_vpro(cfg, bedrock, VPRO_WIDGET, bedrock.interrupt_sender(VPRO_WIDGET, None), &status, |vpro| Ok(xbridge.attach_widget(VPRO_WIDGET, vpro)))?;
    // A link reset of the V12's port resets the board behind it.
    let reset_vpro = Arc::downgrade(&vpro);
    xbridge.set_reset_output(Arc::new(move |port| {
        if port == VPRO_WIDGET {
            if let Some(vpro) = reset_vpro.upgrade() {
                vpro.power_on();
            }
        }
    }));
    eprintln!("iris: {} on the V-brick at widget {VPRO_WIDGET}", cfg.graphics_board().label());
    xbridge.register_locks();
    io.display = Some(vpro.display());
    io.status = Some(status);
    io.monitor.push(vpro.clone());
    io.devices.push(vpro.clone());
    let base_reset = std::mem::replace(&mut io.reset, Box::new(|| {}));
    io.reset = Box::new(move || {
        base_reset();
        xbridge.power_on();
        vpro.power_on();
    });
    Ok(())
}

/// An InfiniteReality pipe at widget 11 of the I-brick's XBridge: DMA enters Bedrock as that
/// widget's inbound traffic, interrupt packets go to the address the driver programs, defaulting to
/// the host port.
fn attach_ir3(
    bedrock: &Arc<Bedrock>,
    xbridge: &Arc<XBridge>,
    cfg: &MachineConfig,
    kona: KonaConfig,
    status: &Arc<crate::disp::StatusSource>,
) -> Result<Arc<Kona>, String> {
    let kona = kona.with_host_widget(HOST_WIDGET)?.with_widget(PIPE_WIDGET)?.with_pool(cfg.graphics.threads, cfg.graphics.tile);
    let flash = cfg.graphics.flash.as_deref().map(|p| crate::dev::kona::flash_file(Path::new(p))).transpose()?;
    // Its interrupt packets are the chip's inbound writes from that widget, to
    // the address the driver programmed.
    let pipe = Kona::new(kona, bedrock.inbound(PIPE_WIDGET), bedrock.inbound(PIPE_WIDGET), status.clone(), flash)?;
    bedrock.io.connect_widget(PIPE_WIDGET, xbridge.attach_widget(PIPE_WIDGET, pipe.clone()));
    Ok(pipe)
}

#[cfg(test)]
mod tests;
