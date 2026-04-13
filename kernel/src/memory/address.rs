// memory/address.rs — Newtype wrappers for physical and virtual addresses.
//
// Using distinct types prevents the common C mistake of passing a virtual address
// where a physical one is required (e.g., into a page table entry).
// The compiler enforces the distinction at zero runtime cost.

/// A physical address (suitable for page table entries, DMA, MMIO).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysicalAddress(u64);

/// A virtual address (used by the CPU when the MMU is active).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct VirtualAddress(u64);

impl PhysicalAddress {
    #[inline]
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// Convert to the HHDM-mapped virtual address for this physical page.
    ///
    /// Valid only while the HHDM is mapped (from early boot onwards).
    #[inline]
    pub fn to_virtual(self, hhdm_offset: u64) -> VirtualAddress {
        VirtualAddress(self.0 + hhdm_offset)
    }

    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Align down to the given power-of-two boundary.
    #[inline]
    pub fn align_down(self, alignment: u64) -> Self {
        debug_assert!(alignment.is_power_of_two());
        Self(self.0 & !(alignment - 1))
    }

    /// Align up to the given power-of-two boundary.
    #[inline]
    pub fn align_up(self, alignment: u64) -> Self {
        debug_assert!(alignment.is_power_of_two());
        Self((self.0 + alignment - 1) & !(alignment - 1))
    }

    #[inline]
    pub fn add(self, offset: u64) -> Self {
        Self(self.0 + offset)
    }
}

impl VirtualAddress {
    #[inline]
    pub const fn new(addr: u64) -> Self {
        Self(addr)
    }

    /// Convert back to a physical address by subtracting the HHDM offset.
    ///
    /// Only valid for addresses that were obtained via `PhysicalAddress::to_virtual`.
    #[inline]
    pub fn to_physical(self, hhdm_offset: u64) -> PhysicalAddress {
        PhysicalAddress(self.0 - hhdm_offset)
    }

    /// Interpret this virtual address as a raw mutable pointer.
    ///
    /// # Safety
    /// The address must be mapped and aligned for `T`.
    #[inline]
    pub fn as_ptr<T>(self) -> *mut T {
        self.0 as *mut T
    }

    #[inline]
    pub fn as_u64(self) -> u64 {
        self.0
    }

    #[inline]
    pub fn align_up(self, alignment: u64) -> Self {
        debug_assert!(alignment.is_power_of_two());
        Self((self.0 + alignment - 1) & !(alignment - 1))
    }

    #[inline]
    pub fn add(self, offset: u64) -> Self {
        Self(self.0 + offset)
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Round `value` up to the nearest multiple of `alignment` (must be power of two).
#[inline]
pub fn align_up(value: u64, alignment: u64) -> u64 {
    debug_assert!(alignment.is_power_of_two());
    (value + alignment - 1) & !(alignment - 1)
}

/// Round `value` down to the nearest multiple of `alignment` (must be power of two).
#[inline]
pub fn align_down(value: u64, alignment: u64) -> u64 {
    debug_assert!(alignment.is_power_of_two());
    value & !(alignment - 1)
}
