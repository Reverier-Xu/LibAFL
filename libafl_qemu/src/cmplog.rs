#[cfg(emulation_mode = "usermode")]
use capstone::{arch::BuildsCapstone, Capstone, InsnDetail};
use hashbrown::HashMap;
use libafl::{inputs::UsesInput, state::HasMetadata};
pub use libafl_targets::{
    cmps::{
        __libafl_targets_cmplog_instructions, __libafl_targets_cmplog_routines, CMPLOG_ENABLED,
    },
    CmpLogMap, CmpLogObserver, CMPLOG_MAP_H, CMPLOG_MAP_PTR, CMPLOG_MAP_SIZE, CMPLOG_MAP_W,
};
use serde::{Deserialize, Serialize};

#[cfg(emulation_mode = "usermode")]
use crate::{
    capstone,
    emu::{ArchExtras, Emulator},
    CallingConvention,
};
use crate::{
    helper::{
        hash_me, HasInstrumentationFilter, QemuHelper, QemuHelperTuple, QemuInstrumentationFilter,
    },
    hooks::QemuHooks,
    GuestAddr,
};

#[cfg_attr(
    any(not(feature = "serdeany_autoreg"), miri),
    allow(clippy::unsafe_derive_deserialize)
)] // for SerdeAny
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct QemuCmpsMapMetadata {
    pub map: HashMap<u64, u64>,
    pub current_id: u64,
}

impl QemuCmpsMapMetadata {
    #[must_use]
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            current_id: 0,
        }
    }
}

libafl_bolts::impl_serdeany!(QemuCmpsMapMetadata);

#[derive(Debug)]
pub struct QemuCmpLogHelper {
    filter: QemuInstrumentationFilter,
}

impl QemuCmpLogHelper {
    #[must_use]
    pub fn new(filter: QemuInstrumentationFilter) -> Self {
        Self { filter }
    }

    #[must_use]
    pub fn must_instrument(&self, addr: GuestAddr) -> bool {
        self.filter.allowed(addr)
    }
}

impl Default for QemuCmpLogHelper {
    fn default() -> Self {
        Self::new(QemuInstrumentationFilter::None)
    }
}

impl HasInstrumentationFilter for QemuCmpLogHelper {
    fn filter(&self) -> &QemuInstrumentationFilter {
        &self.filter
    }

    fn filter_mut(&mut self) -> &mut QemuInstrumentationFilter {
        &mut self.filter
    }
}

impl<S> QemuHelper<S> for QemuCmpLogHelper
where
    S: UsesInput + HasMetadata,
{
    fn first_exec<QT>(&self, hooks: &QemuHooks<'_, QT, S>)
    where
        QT: QemuHelperTuple<S>,
    {
        hooks.cmps_raw(
            Some(gen_unique_cmp_ids::<QT, S>),
            Some(trace_cmp1_cmplog),
            Some(trace_cmp2_cmplog),
            Some(trace_cmp4_cmplog),
            Some(trace_cmp8_cmplog),
        );
    }
}

#[derive(Debug)]
pub struct QemuCmpLogChildHelper {
    filter: QemuInstrumentationFilter,
}

impl QemuCmpLogChildHelper {
    #[must_use]
    pub fn new(filter: QemuInstrumentationFilter) -> Self {
        Self { filter }
    }

    #[must_use]
    pub fn must_instrument(&self, addr: GuestAddr) -> bool {
        self.filter.allowed(addr)
    }
}

impl Default for QemuCmpLogChildHelper {
    fn default() -> Self {
        Self::new(QemuInstrumentationFilter::None)
    }
}

impl<S> QemuHelper<S> for QemuCmpLogChildHelper
where
    S: UsesInput,
    S: HasMetadata,
{
    const HOOKS_DO_SIDE_EFFECTS: bool = false;

    fn first_exec<QT>(&self, hooks: &QemuHooks<'_, QT, S>)
    where
        QT: QemuHelperTuple<S>,
    {
        hooks.cmps_raw(
            Some(gen_hashed_cmp_ids::<QT, S>),
            Some(trace_cmp1_cmplog),
            Some(trace_cmp2_cmplog),
            Some(trace_cmp4_cmplog),
            Some(trace_cmp8_cmplog),
        );
    }
}

pub fn gen_unique_cmp_ids<QT, S>(
    hooks: &mut QemuHooks<'_, QT, S>,
    state: Option<&mut S>,
    pc: GuestAddr,
    _size: usize,
) -> Option<u64>
where
    S: HasMetadata,
    S: UsesInput,
    QT: QemuHelperTuple<S>,
{
    if let Some(h) = hooks.match_helper_mut::<QemuCmpLogHelper>() {
        if !h.must_instrument(pc) {
            return None;
        }
    }
    let state = state.expect("The gen_unique_cmp_ids hook works only for in-process fuzzing");
    if state.metadata_map().get::<QemuCmpsMapMetadata>().is_none() {
        state.add_metadata(QemuCmpsMapMetadata::new());
    }
    let meta = state
        .metadata_map_mut()
        .get_mut::<QemuCmpsMapMetadata>()
        .unwrap();
    let id = meta.current_id as usize;

    Some(*meta.map.entry(pc.into()).or_insert_with(|| {
        meta.current_id = ((id + 1) & (CMPLOG_MAP_W - 1)) as u64;
        id as u64
    }))
}

pub fn gen_hashed_cmp_ids<QT, S>(
    hooks: &mut QemuHooks<'_, QT, S>,
    _state: Option<&mut S>,
    pc: GuestAddr,
    _size: usize,
) -> Option<u64>
where
    S: HasMetadata,
    S: UsesInput,
    QT: QemuHelperTuple<S>,
{
    if let Some(h) = hooks.match_helper_mut::<QemuCmpLogChildHelper>() {
        if !h.must_instrument(pc) {
            return None;
        }
    }
    Some(hash_me(pc.into()) & (CMPLOG_MAP_W as u64 - 1))
}

pub extern "C" fn trace_cmp1_cmplog(id: u64, v0: u8, v1: u8, _data: u64) {
    unsafe {
        __libafl_targets_cmplog_instructions(id as usize, 1, u64::from(v0), u64::from(v1));
    }
}

pub extern "C" fn trace_cmp2_cmplog(id: u64, v0: u16, v1: u16, _data: u64) {
    unsafe {
        __libafl_targets_cmplog_instructions(id as usize, 2, u64::from(v0), u64::from(v1));
    }
}

pub extern "C" fn trace_cmp4_cmplog(id: u64, v0: u32, v1: u32, _data: u64) {
    unsafe {
        __libafl_targets_cmplog_instructions(id as usize, 4, u64::from(v0), u64::from(v1));
    }
}

pub extern "C" fn trace_cmp8_cmplog(id: u64, v0: u64, v1: u64, _data: u64) {
    unsafe {
        __libafl_targets_cmplog_instructions(id as usize, 8, v0, v1);
    }
}

#[cfg(emulation_mode = "usermode")]
#[derive(Debug)]
pub struct QemuCmpLogRoutinesHelper {
    filter: QemuInstrumentationFilter,
    cs: Capstone,
}

#[cfg(emulation_mode = "usermode")]
impl QemuCmpLogRoutinesHelper {
    #[must_use]
    pub fn new(filter: QemuInstrumentationFilter) -> Self {
        Self {
            filter,
            cs: capstone().detail(true).build().unwrap(),
        }
    }

    #[must_use]
    pub fn must_instrument(&self, addr: GuestAddr) -> bool {
        self.filter.allowed(addr)
    }

    extern "C" fn on_call(_pc: GuestAddr, k: u64) {
        unsafe {
            if CMPLOG_ENABLED == 0 {
                return;
            }
        }

        let emu = Emulator::new_empty();

        let a0: GuestAddr = emu
            .read_function_argument(CallingConvention::Cdecl, 0)
            .unwrap_or(0);
        let a1: GuestAddr = emu
            .read_function_argument(CallingConvention::Cdecl, 1)
            .unwrap_or(0);

        if a0 == 0 || a1 == 0 {
            return;
        }

        // if !emu.access_ok(VerifyAccess::Read, a0, 0x20) || !emu.access_ok(VerifyAccess::Read, a1, 0x20) { return; }

        unsafe {
            __libafl_targets_cmplog_routines(k as usize, emu.g2h(a0), emu.g2h(a1));
        }
    }

    fn gen_blocks_calls<QT, S>(
        hooks: &mut QemuHooks<'_, QT, S>,
        _state: Option<&mut S>,
        pc: GuestAddr,
    ) -> Option<u64>
    where
        S: UsesInput,
        QT: QemuHelperTuple<S>,
    {
        if let Some(h) = hooks.helpers_mut().match_first_type_mut::<Self>() {
            if !h.must_instrument(pc) {
                return None;
            }

            #[cfg(cpu_target = "arm")]
            h.cs.set_mode(if pc & 1 == 1 {
                capstone::arch::arm::ArchMode::Thumb.into()
            } else {
                capstone::arch::arm::ArchMode::Arm.into()
            })
            .unwrap();
        }

        let emu = hooks.emulator();

        if let Some(h) = hooks.helpers().match_first_type::<Self>() {
            #[allow(unused_mut)]
            let mut code = {
                #[cfg(emulation_mode = "usermode")]
                unsafe {
                    std::slice::from_raw_parts(emu.g2h(pc), 512)
                }
                #[cfg(emulation_mode = "systemmode")]
                &mut [0; 512]
            };
            #[cfg(emulation_mode = "systemmode")]
            unsafe {
                emu.read_mem(pc, code)
            }; // TODO handle faults

            let mut iaddr = pc;

            'disasm: while let Ok(insns) = h.cs.disasm_count(code, iaddr.into(), 1) {
                if insns.is_empty() {
                    break;
                }
                let insn = insns.first().unwrap();
                let insn_detail: InsnDetail = h.cs.insn_detail(insn).unwrap();
                for detail in insn_detail.groups() {
                    match u32::from(detail.0) {
                        capstone::InsnGroupType::CS_GRP_CALL => {
                            let k = (hash_me(pc.into())) & (CMPLOG_MAP_W as u64 - 1);
                            emu.set_hook(insn.address() as GuestAddr, Self::on_call, k, false);
                        }
                        capstone::InsnGroupType::CS_GRP_RET
                        | capstone::InsnGroupType::CS_GRP_INVALID
                        | capstone::InsnGroupType::CS_GRP_JUMP
                        | capstone::InsnGroupType::CS_GRP_IRET
                        | capstone::InsnGroupType::CS_GRP_PRIVILEGE => {
                            break 'disasm;
                        }
                        _ => {}
                    }
                }

                iaddr += insn.bytes().len() as GuestAddr;

                #[cfg(emulation_mode = "usermode")]
                unsafe {
                    code = std::slice::from_raw_parts(emu.g2h(iaddr), 512);
                }
                #[cfg(emulation_mode = "systemmode")]
                unsafe {
                    emu.read_mem(pc, code);
                } // TODO handle faults
            }
        }

        None
    }
}

#[cfg(emulation_mode = "usermode")]
impl HasInstrumentationFilter for QemuCmpLogRoutinesHelper {
    fn filter(&self) -> &QemuInstrumentationFilter {
        &self.filter
    }

    fn filter_mut(&mut self) -> &mut QemuInstrumentationFilter {
        &mut self.filter
    }
}

#[cfg(emulation_mode = "usermode")]
impl<S> QemuHelper<S> for QemuCmpLogRoutinesHelper
where
    S: UsesInput,
{
    fn first_exec<QT>(&self, hooks: &QemuHooks<'_, QT, S>)
    where
        QT: QemuHelperTuple<S>,
    {
        hooks.blocks(Some(Self::gen_blocks_calls::<QT, S>), None, None);
    }
}
