#pragma once

#include <stdint.h>

// ---------------------------------------------------------------------------
// FAT32 on-disk structures — FAT 32 Spec (Microsoft, VFD 1.03)
//
// All structures are little-endian and __attribute__((packed)).
// ---------------------------------------------------------------------------

// --- BIOS Parameter Block (BPB) — starts at offset 3 of the boot sector ---

typedef struct {
    uint8_t  jmp_boot[3];         // 0xEB 0x58 0x90 or 0xE9 0x?? 0x90
    uint8_t  oem_name[8];         // e.g. "MSDOS5.0"
    // --- BPB ---
    uint16_t bytes_per_sec;       // always 512 for FAT32
    uint8_t  sec_per_clus;        // sectors per cluster (power of 2)
    uint16_t rsvd_sec_cnt;        // reserved sector count (includes boot sector)
    uint8_t  num_fats;            // number of FATs (usually 2)
    uint16_t root_ent_cnt;       // 0 for FAT32
    uint16_t tot_sec16;          // 0 for FAT32 (use tot_sec32)
    uint8_t  media;               // media descriptor (0xF8 = hard disk)
    uint16_t fat_sz16;           // 0 for FAT32
    uint16_t sec_per_trk;        // sectors per track
    uint16_t num_heads;          // number of heads
    uint32_t hidd_sec;           // hidden sectors
    uint32_t tot_sec32;          // total sectors (32-bit)
    // --- FAT32 EBPB ---
    uint32_t fat_sz32;           // sectors per FAT
    uint16_t ext_flags;          // mirroring flags
    uint16_t fs_ver;             // filesystem version (0:0)
    uint32_t root_clus;          // cluster number of root directory
    uint16_t fs_info;            // sector number of FSInfo structure
    uint16_t bk_boot_sec;        // sector number of backup boot sector
    uint8_t  reserved[12];       // reserved, should be 0
    uint8_t  drv_num;            // drive number (0x80 = hard disk)
    uint8_t  reserved1;          // reserved
    uint8_t  boot_sig;            // extended boot signature (0x29)
    uint32_t vol_id;             // volume ID
    uint8_t  vol_lab[11];        // volume label
    uint8_t  fil_sys_type[8];    // "FAT32   "
} __attribute__((packed)) fat32_bpb_t;

// --- FAT32 Directory Entry (32 bytes) ---

// Short name entry (type 0x00 or 0xE5 for deleted)
typedef struct {
    uint8_t  name[11];           // 8.3 name (space-padded, no dot)
    uint8_t  attr;               // attribute bits
    uint8_t  nt_res;             // NT reserved (case flags)
    uint8_t  ctime_cs;           // create time, 10ms units (0-199)
    uint16_t c_time;             // create time (HH:MM:SS/2)
    uint16_t c_date;             // create date (YYYY/M/D)
    uint16_t a_date;             // last access date
    uint16_t fst_clus_hi;        // high 16 bits of first cluster
    uint16_t w_time;             // last write time
    uint16_t w_date;             // last write date
    uint16_t fst_clus_lo;        // low 16 bits of first cluster
    uint32_t file_size;          // file size in bytes
} __attribute__((packed)) fat32_dir_entry_t;

// Long File Name entry (VFAT extension)
typedef struct {
    uint8_t  ord;                // order + flags (0x40 = last LFN entry)
    uint16_t name1[5];           // characters 1-5
    uint8_t  attr;               // always 0x0F for LFN
    uint8_t  type;               // 0x00 (must be)
    uint8_t  chksum;             // checksum of short name
    uint16_t name2[6];           // characters 6-11
    uint16_t fst_clus_lo;        // always 0 for LFN
    uint16_t name3[2];           // characters 12-13
} __attribute__((packed)) fat32_lfn_entry_t;

// --- Attribute bits ---
#define FAT32_ATTR_READ_ONLY   0x01
#define FAT32_ATTR_HIDDEN       0x02
#define FAT32_ATTR_SYSTEM       0x04
#define FAT32_ATTR_VOLUME_ID    0x08
#define FAT32_ATTR_DIRECTORY    0x10
#define FAT32_ATTR_ARCHIVE      0x20
#define FAT32_ATTR_LONG_NAME    0x0F  // LFN mask

// --- Directory entry name[0] special values ---
#define FAT32_DIR_ENTRY_FREE   0x00  // end of directory
#define FAT32_DIR_ENTRY_DELETED 0xE5 // deleted entry

// --- Cluster constants ---
#define FAT32_CLUSTER_EOF      0x0FFFFFF8  // EOF marker (upper 4 bits = 0x0F)
#define FAT32_CLUSTER_BAD      0x0FFFFFF7  // bad cluster
#define FAT32_CLUSTER_FREE     0x00000000  // free cluster
#define FAT32_FIRST_DATA_CLUSTER 2

// --- Helpers ---

// Extract the 32-bit cluster number from a directory entry.
static inline uint32_t fat32_entry_cluster(const fat32_dir_entry_t *entry)
{
    return ((uint32_t)entry->fst_clus_hi << 16) | (uint32_t)entry->fst_clus_lo;
}

// Check if a cluster value marks EOF.
static inline int fat32_is_eof(uint32_t cluster)
{
    return (cluster & 0x0FFFFFF8) == 0x0FFFFFF8;
}

// Check if a cluster value marks a bad cluster.
static inline int fat32_is_bad(uint32_t cluster)
{
    return cluster == FAT32_CLUSTER_BAD;
}

// Check if an attribute byte indicates a long file name entry.
static inline int fat32_is_lfn(const fat32_dir_entry_t *entry)
{
    return entry->attr == FAT32_ATTR_LONG_NAME;
}

// Maximum LFN entries in a chain.
#define FAT32_MAX_LFN_ENTRIES 20
// Maximum LFN characters (13 per entry * 20 entries).
#define FAT32_MAX_LFN_LEN     (FAT32_MAX_LFN_ENTRIES * 13)
