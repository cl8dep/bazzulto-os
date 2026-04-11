#!/usr/bin/env python3
"""Create a FAT32 disk image with test files for Bazzulto OS."""

import struct
import sys
import os
import math

SECTOR = 512
TOTAL_SECTORS = 2097152  # 1 GB
SEC_PER_CLUSTER = 8      # 4KB clusters
NUM_FATS = 2
RESERVED_SECTORS = 32
ROOT_CLUSTER = 2
VOLUME_LABEL = "BAZZULTO   "

def create_fat32_image(output_path, size_mb=1024):
    total_sectors = size_mb * 1024 * 1024 // SECTOR
    sec_per_cluster = 8
    fat_sectors = math.ceil(total_sectors * 4 / SECTOR)  # rough estimate
    # More precise: fat_sectors = ceil((total_sectors - reserved - 2*fat) * 4 / SECTOR)
    # Iterative solution:
    reserved = 32
    # Initial guess
    fat_sectors = (total_sectors * 4) // SECTOR + 1
    # Iterate to converge
    for _ in range(10):
        data_sectors = total_sectors - reserved - NUM_FATS * fat_sectors
        total_clusters = data_sectors // sec_per_cluster
        fat_sectors = math.ceil((total_clusters + 2) * 4 / SECTOR)
    
    data_sectors = total_sectors - reserved - NUM_FATS * fat_sectors
    total_clusters = data_sectors // sec_per_cluster
    
    print(f"Image size: {size_mb}MB, {total_sectors} sectors")
    print(f"Reserved: {reserved}, FAT sectors: {fat_sectors}, Data sectors: {data_sectors}")
    print(f"Total clusters: {total_clusters}")
    
    # Create the image buffer
    image = bytearray(total_sectors * SECTOR)
    
    # --- Boot sector (sector 0) ---
    bs = bytearray(SECTOR)
    
    # Jump instruction
    bs[0] = 0xEB
    bs[1] = 0x58
    bs[2] = 0x90
    
    # OEM name
    bs[3:11] = b"MSDOS5.0"
    
    # BPB
    struct.pack_into('<H', bs, 11, SECTOR)           # bytes_per_sec = 512
    bs[13] = sec_per_cluster                          # sec_per_clus
    struct.pack_into('<H', bs, 14, reserved)          # rsvd_sec_cnt
    bs[16] = NUM_FATS                                 # num_fats
    struct.pack_into('<H', bs, 17, 0)                 # root_ent_cnt = 0 for FAT32
    struct.pack_into('<H', bs, 19, 0)                 # tot_sec16 = 0 for FAT32
    bs[21] = 0xF8                                     # media = hard disk
    struct.pack_into('<H', bs, 22, 0)                 # fat_sz16 = 0 for FAT32
    struct.pack_into('<H', bs, 24, 0x20)              # sec_per_trk
    struct.pack_into('<H', bs, 26, 0x40)              # num_heads
    struct.pack_into('<I', bs, 28, 0)                 # hidd_sec = 0
    struct.pack_into('<I', bs, 32, total_sectors)     # tot_sec32
    
    # FAT32 EBPB
    struct.pack_into('<I', bs, 36, fat_sectors)       # fat_sz32
    struct.pack_into('<H', bs, 40, 0)                 # ext_flags
    struct.pack_into('<H', bs, 42, 0)                 # fs_ver = 0
    struct.pack_into('<I', bs, 44, ROOT_CLUSTER)      # root_clus
    struct.pack_into('<H', bs, 48, 1)                 # fs_info sector = 1
    struct.pack_into('<H', bs, 50, 6)                 # bk_boot_sec = 6
    
    bs[64] = 0x80                                     # drv_num
    bs[66] = 0x29                                     # boot_sig
    struct.pack_into('<I', bs, 67, 0x12345678)        # vol_id
    bs[71:82] = VOLUME_LABEL.encode('ascii')          # vol_lab
    bs[82:90] = b"FAT32   "                           # fil_sys_type
    
    # Boot signature
    bs[510] = 0x55
    bs[511] = 0xAA
    
    image[0:SECTOR] = bs
    
    # --- Backup boot sector (sector 6) ---
    image[6*SECTOR:7*SECTOR] = bytes(bs)
    
    # --- FSInfo sector (sector 1) ---
    fsinfo = bytearray(SECTOR)
    struct.pack_into('<I', fsinfo, 0, 0x41615252)      # Lead signature
    struct.pack_into('<I', fsinfo, 484, 0x61417272)    # Struct signature
    struct.pack_into('<I', fsinfo, 488, total_clusters - 2)  # Free clusters (exclude root)
    struct.pack_into('<I', fsinfo, 492, ROOT_CLUSTER + 1)     # Next free cluster
    struct.pack_into('<I', fsinfo, 508, 0xAA550000)    # Tail signature (bytes 508-511)
    
    image[SECTOR:2*SECTOR] = bytes(fsinfo)
    
    # --- FAT tables ---
    fat_start = reserved * SECTOR
    fat_data = bytearray(fat_sectors * SECTOR)
    
    # Cluster 0: media type + reserved
    struct.pack_into('<I', fat_data, 0, 0x0FFFFFF8)   # cluster 0 = media descriptor
    struct.pack_into('<I', fat_data, 4, 0x0FFFFFFF)   # cluster 1 = reserved
    # Root cluster (2) = EOF
    struct.pack_into('<I', fat_data, 8, 0x0FFFFFFF)   # cluster 2 = EOF (empty root dir)
    
    # Write both FAT copies
    for fat in range(NUM_FATS):
        offset = fat_start + fat * fat_sectors * SECTOR
        image[offset:offset + len(fat_data)] = bytes(fat_data)
    
    # --- Root directory (cluster 2) ---
    root_lba = reserved + NUM_FATS * fat_sectors  # first data sector
    root_offset = root_lba * SECTOR
    # Root directory starts empty (just free markers)
    # Actually it should be all zeros (no entries)
    # The first free entry marker (0x00) is at offset 0, meaning empty directory
    
    # --- Add test files ---
    files = []
    
    # File 1: hello.txt
    files.append(("HELLO   TXT", "Hello from Bazzulto OS on FAT32!\n"))
    # File 2: test.txt
    files.append(("TEST    TXT", "This is a test file on the FAT32 disk.\n"))
    # File 3: README.TXT
    files.append(("README  TXT", 
                  "Bazzulto OS FAT32 disk\n"
                  "=====================\n"
                  "This is a test disk image.\n"
                  "Try: cat /mnt/hello.txt\n"
                  "Try: echo test > /mnt/output.txt\n"
                  "Try: cat /mnt/output.txt\n"))
    
    # Allocate clusters for files (one cluster each, they're tiny)
    current_cluster = ROOT_CLUSTER + 1  # start after root dir cluster
    
    dir_offset = root_offset
    for short_name, content in files:
        content_bytes = content.encode('ascii')
        file_size = len(content_bytes)
        
        # Allocate one cluster for the file
        file_cluster = current_cluster
        current_cluster += 1
        
        # Write file data to its cluster
        data_lba = root_lba + (file_cluster - ROOT_CLUSTER) * sec_per_cluster
        data_offset = data_lba * SECTOR
        image[data_offset:data_offset + file_size] = content_bytes
        
        # Update FAT: mark file cluster as EOF
        fat_offset = fat_start + file_cluster * 4
        struct.pack_into('<I', fat_data, fat_offset - fat_start, 0x0FFFFFFF)
        
        # Write FAT copies
        for fat in range(NUM_FATS):
            offset = fat_start + fat * fat_sectors * SECTOR
            image[offset:offset + len(fat_data)] = bytes(fat_data)
        
        # Create directory entry
        entry = bytearray(32)
        entry[0:11] = short_name.encode('ascii')
        entry[11] = 0x20  # archive attribute
        # Time/date: set to some reasonable value
        struct.pack_into('<H', entry, 22, (file_cluster >> 16) & 0xFFFF)  # fst_clus_hi
        struct.pack_into('<H', entry, 26, file_cluster & 0xFFFF)          # fst_clus_lo
        struct.pack_into('<I', entry, 28, file_size)                       # file_size
        
        image[dir_offset:dir_offset+32] = bytes(entry)
        dir_offset += 32
    
    # Write the image to disk
    with open(output_path, 'wb') as f:
        f.write(image)
    
    print(f"Wrote {len(files)} files to {output_path}")
    for name, content in files:
        print(f"  {name.strip()}: {len(content.encode('ascii'))} bytes, cluster {ROOT_CLUSTER + 1 + files.index((name, content))}")

if __name__ == '__main__':
    out = sys.argv[1] if len(sys.argv) > 1 else 'disk.img'
    create_fat32_image(out, size_mb=1024)
