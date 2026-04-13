// platform/dtb.rs — Flattened Device Tree (FDT) binary parser.
//
// Parses the FDT v0.3 binary format to extract hardware topology from a
// Device Tree Blob (DTB) provided by the bootloader or firmware.
//
// All multi-byte values in the FDT binary are big-endian.
//
// Reference: Devicetree Specification v0.3 §5 (Flattened Devicetree Format).

// ---------------------------------------------------------------------------
// FDT header constants
// Reference: Devicetree Specification v0.3 §5.2
// ---------------------------------------------------------------------------

/// Expected FDT magic value at offset 0 in the blob.
const FDT_MAGIC: u32 = 0xd00dfeed;

/// FDT structure block token: begin a new node.
const FDT_BEGIN_NODE: u32 = 1;
/// FDT structure block token: end the current node.
const FDT_END_NODE: u32 = 2;
/// FDT structure block token: property record follows.
const FDT_PROP: u32 = 3;
/// FDT structure block token: no-operation; skip.
const FDT_NOP: u32 = 4;
/// FDT structure block token: end of structure block.
const FDT_END: u32 = 9;

// ---------------------------------------------------------------------------
// DtbInfo — parsed platform description
// ---------------------------------------------------------------------------

/// Platform hardware description extracted from a Device Tree Blob.
pub struct DtbInfo {
    /// Physical base address of the PL011-compatible UART.
    pub uart_phys_base: u64,
    /// Physical base address of the GIC Distributor (GICD).
    pub gicd_phys_base: u64,
    /// Physical base address of the GICv2 CPU Interface (GICC). 0 for GICv3.
    pub gicc_phys_base: u64,
    /// Physical base address of the GICv3 Redistributor (GICR). 0 for GICv2.
    pub gicr_phys_base: u64,
    /// GIC architecture version: 2 or 3.
    pub gic_version: u8,
    /// ARM Generic Timer EL1 physical timer PPI INTID.
    /// Architectural constant — always 30, per ARM ARM DDI 0487 D11.2 Table D11-1.
    pub timer_intid: u32,
    /// Total installed RAM in bytes (sum of all /memory regions).
    pub total_memory_bytes: u64,
    /// Number of CPU cores found in /cpus.
    pub cpu_count: u32,
}

impl DtbInfo {
    /// Defaults matching QEMU virt — used as fallback if no DTB is present or
    /// if parsing fails.
    ///
    /// Physical addresses verified from the QEMU-generated DTB for this repo's
    /// machine configuration (see CLAUDE.md "Verified machine facts").
    pub const fn qemu_virt_defaults() -> Self {
        Self {
            uart_phys_base: 0x09000000,
            gicd_phys_base: 0x08000000,
            gicc_phys_base: 0x08010000,
            gicr_phys_base: 0,
            gic_version: 2,
            // ARM Generic Timer EL1 physical timer PPI INTID.
            // Architectural constant — ARM ARM DDI 0487 D11.2 Table D11-1.
            timer_intid: 30,
            total_memory_bytes: 0,
            cpu_count: 1,
        }
    }
}

// ---------------------------------------------------------------------------
// FDT header layout
// Reference: Devicetree Specification v0.3 §5.2
// ---------------------------------------------------------------------------

/// Read a big-endian u32 from a raw byte pointer at the given byte offset.
///
/// # Safety
/// `base` must be a valid pointer to at least `offset + 4` readable bytes.
#[inline]
unsafe fn read_be_u32(base: *const u8, offset: usize) -> u32 {
    let ptr = base.add(offset) as *const [u8; 4];
    u32::from_be_bytes(*ptr)
}

/// Read a big-endian u64 from a raw byte pointer at the given byte offset.
///
/// # Safety
/// `base` must be a valid pointer to at least `offset + 8` readable bytes.
#[inline]
unsafe fn read_be_u64(base: *const u8, offset: usize) -> u64 {
    let ptr = base.add(offset) as *const [u8; 8];
    u64::from_be_bytes(*ptr)
}

/// Round `value` up to the nearest multiple of 4.
#[inline]
const fn align4(value: usize) -> usize {
    (value + 3) & !3
}

/// Read a NUL-terminated string starting at `ptr`, returning (str, bytes_consumed).
/// `bytes_consumed` includes the terminating NUL but is NOT padded to alignment.
///
/// # Safety
/// `ptr` must point into readable memory and be NUL-terminated within bounds.
unsafe fn read_cstr(ptr: *const u8) -> &'static [u8] {
    let mut len = 0usize;
    while *ptr.add(len) != 0 {
        len += 1;
    }
    core::slice::from_raw_parts(ptr, len)
}

/// Check whether a `compatible` property value (which may contain multiple
/// NUL-separated strings) contains `needle`.
fn compatible_contains(data: &[u8], needle: &[u8]) -> bool {
    let mut i = 0usize;
    while i < data.len() {
        // Find the end of this NUL-terminated entry.
        let end = data[i..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| i + p)
            .unwrap_or(data.len());
        let entry = &data[i..end];
        if entry == needle {
            return true;
        }
        i = end + 1; // skip past the NUL
    }
    false
}

// ---------------------------------------------------------------------------
// Public parser
// ---------------------------------------------------------------------------

/// Parse a raw FDT binary starting at `dtb_ptr`.
///
/// Returns `None` if the magic number is wrong or the structure is too
/// malformed to walk safely.  Individual missing properties fall back to
/// the QEMU virt defaults already stored in the returned `DtbInfo`.
///
/// # Safety
/// `dtb_ptr` must point to a valid, readable FDT binary for the lifetime of
/// the call.
pub unsafe fn parse_dtb(dtb_ptr: *const u8) -> Option<DtbInfo> {
    // Validate magic.
    if read_be_u32(dtb_ptr, 0) != FDT_MAGIC {
        return None;
    }

    let total_size   = read_be_u32(dtb_ptr, 4)  as usize;
    let off_struct   = read_be_u32(dtb_ptr, 8)  as usize;
    let off_strings  = read_be_u32(dtb_ptr, 12) as usize;

    // Minimal sanity: offsets must be within the blob.
    if off_struct >= total_size || off_strings >= total_size {
        return None;
    }

    // Maximum bytes available in the structure block (cursor limit).
    let struct_limit: usize = total_size - off_struct;

    let strings_base: *const u8 = dtb_ptr.add(off_strings);
    let struct_base:  *const u8 = dtb_ptr.add(off_struct);

    // Parser cursor (byte offset from struct_base).
    let mut cursor: usize = 0;

    // Accumulated results — start from defaults.
    let mut result = DtbInfo::qemu_virt_defaults();
    result.total_memory_bytes = 0; // will accumulate from /memory nodes
    result.cpu_count = 0;          // will count from /cpus children

    // Node depth stack:
    //   0 = root, 1 = top-level nodes, 2 = children, …
    //
    // We track the current node name and depth to identify /memory, /cpus,
    // and specific compatible device nodes.
    //
    // We use a simple fixed-depth scheme: only track name at depth 1 and 2.
    let mut depth: u32 = 0;

    // Name of the most-recently-opened node at depth 1 (e.g. "memory@…").
    let mut depth1_name: [u8; 64] = [0u8; 64];
    let mut depth1_name_len: usize = 0;

    // Pending compatible string for the current node (last seen value).
    let mut current_compatible: [u8; 256] = [0u8; 256];
    let mut current_compatible_len: usize = 0;

    // Pending reg property for the current node.
    let mut current_reg: [u8; 64] = [0u8; 64];
    let mut current_reg_len: usize = 0;

    // Whether we are inside the /cpus node (depth 1).
    let mut inside_cpus: bool = false;

    // Helper: read a token (u32 BE) and advance the cursor.
    macro_rules! next_token {
        () => {{
            if cursor + 4 > struct_limit {
                return Some(result);
            }
            let token = read_be_u32(struct_base, cursor);
            cursor += 4;
            token
        }};
    }

    loop {
        let token = next_token!();

        match token {
            FDT_NOP => {
                // No-op: advance to the next token.
            }

            FDT_BEGIN_NODE => {
                // Read the NUL-terminated node name and advance past it
                // (padded to 4-byte alignment from the start of the name).
                let name_start = cursor;
                let name_bytes = read_cstr(struct_base.add(cursor));
                let name_len   = name_bytes.len();
                cursor = name_start + align4(name_len + 1); // +1 for NUL

                depth += 1;

                if depth == 1 {
                    // Save the top-level node name for /memory and /cpus detection.
                    let copy_len = name_len.min(depth1_name.len());
                    depth1_name[..copy_len].copy_from_slice(&name_bytes[..copy_len]);
                    depth1_name_len = copy_len;

                    inside_cpus = &depth1_name[..depth1_name_len] == b"cpus";
                }

                // If entering a direct child of /cpus, count it as a CPU core.
                if depth == 2 && inside_cpus {
                    // Only count nodes whose name starts with "cpu@" or "cpu\0".
                    let is_cpu = name_len >= 3 && &name_bytes[..3] == b"cpu";
                    if is_cpu {
                        result.cpu_count += 1;
                    }
                }

                // Reset per-node properties when entering a new node.
                current_compatible_len = 0;
                current_reg_len        = 0;
            }

            FDT_END_NODE => {
                // Apply accumulated properties before leaving this node.
                apply_node_properties(
                    depth,
                    &depth1_name[..depth1_name_len],
                    &current_compatible[..current_compatible_len],
                    &current_reg[..current_reg_len],
                    &mut result,
                );

                if depth == 1 {
                    inside_cpus = false;
                    depth1_name_len = 0;
                }

                current_compatible_len = 0;
                current_reg_len        = 0;

                if depth > 0 {
                    depth -= 1;
                }
            }

            FDT_PROP => {
                // Struct: { len: u32 BE, nameoff: u32 BE } followed by `len` bytes.
                if cursor + 8 > struct_limit {
                    return Some(result);
                }
                let prop_len     = read_be_u32(struct_base, cursor) as usize;
                let prop_nameoff = read_be_u32(struct_base, cursor + 4) as usize;
                cursor += 8;

                // Resolve property name from the strings block.
                let prop_name = read_cstr(strings_base.add(prop_nameoff));

                // Capture relevant properties.
                if prop_name == b"compatible" {
                    let copy_len = prop_len.min(current_compatible.len());
                    core::ptr::copy_nonoverlapping(
                        struct_base.add(cursor),
                        current_compatible.as_mut_ptr(),
                        copy_len,
                    );
                    current_compatible_len = copy_len;
                } else if prop_name == b"reg" {
                    let copy_len = prop_len.min(current_reg.len());
                    core::ptr::copy_nonoverlapping(
                        struct_base.add(cursor),
                        current_reg.as_mut_ptr(),
                        copy_len,
                    );
                    current_reg_len = copy_len;
                }

                // Advance past the property data (padded to 4 bytes).
                cursor += align4(prop_len);
            }

            FDT_END => {
                break;
            }

            _ => {
                // Unknown token — the blob is malformed; stop parsing.
                break;
            }
        }
    }

    // Ensure cpu_count is at least 1.
    if result.cpu_count == 0 {
        result.cpu_count = 1;
    }

    Some(result)
}

// ---------------------------------------------------------------------------
// Node property application
// ---------------------------------------------------------------------------

/// Apply the accumulated `compatible` and `reg` properties collected while
/// walking a single node.
///
/// This is called on FDT_END_NODE so that both `compatible` and `reg` have
/// been seen for the node.
fn apply_node_properties(
    depth: u32,
    depth1_name: &[u8],
    compatible: &[u8],
    reg: &[u8],
    result: &mut DtbInfo,
) {
    if compatible.is_empty() {
        // /memory nodes use the node name prefix rather than compatible.
        // Apply memory region if this is a /memory node.
        if depth == 1 {
            let is_memory = depth1_name.len() >= 6 && &depth1_name[..6] == b"memory";
            if is_memory && reg.len() >= 16 {
                // Assume 2-cell address (u64 BE) + 2-cell size (u64 BE).
                // Reference: Devicetree Specification v0.3 §2.3.6.
                let size = read_reg_u64(reg, 8);
                result.total_memory_bytes += size;
            }
        }
        return;
    }

    // PL011 UART
    if compatible_contains(compatible, b"arm,pl011") {
        if reg.len() >= 8 {
            result.uart_phys_base = read_reg_u64(reg, 0);
        }
        return;
    }

    // GICv3
    if compatible_contains(compatible, b"arm,gic-v3") {
        result.gic_version = 3;
        result.gicc_phys_base = 0; // GICv3 uses system registers for CPU interface
        if reg.len() >= 8 {
            result.gicd_phys_base = read_reg_u64(reg, 0);
        }
        // GICR is the second region in the reg property.
        // Each region is 2 cells address + 2 cells size = 16 bytes.
        if reg.len() >= 32 {
            result.gicr_phys_base = read_reg_u64(reg, 16);
        }
        return;
    }

    // GICv2 (cortex-a15-gic or gic-400)
    if compatible_contains(compatible, b"arm,cortex-a15-gic")
        || compatible_contains(compatible, b"arm,gic-400")
    {
        result.gic_version = 2;
        result.gicr_phys_base = 0;
        if reg.len() >= 8 {
            result.gicd_phys_base = read_reg_u64(reg, 0);
        }
        // GICC is the second region: 16 bytes in (after first addr+size pair).
        if reg.len() >= 24 {
            result.gicc_phys_base = read_reg_u64(reg, 16);
        }
        return;
    }

    // /memory node with a compatible property (rare, but handle it).
    if depth == 1 {
        let is_memory = depth1_name.len() >= 6 && &depth1_name[..6] == b"memory";
        if is_memory && reg.len() >= 16 {
            let size = read_reg_u64(reg, 8);
            result.total_memory_bytes += size;
        }
    }
}

/// Read a big-endian u64 from `data` at byte offset `offset`.
/// Returns 0 if there are not enough bytes.
fn read_reg_u64(data: &[u8], offset: usize) -> u64 {
    if offset + 8 > data.len() {
        return 0;
    }
    let bytes: [u8; 8] = [
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
        data[offset + 4],
        data[offset + 5],
        data[offset + 6],
        data[offset + 7],
    ];
    u64::from_be_bytes(bytes)
}
