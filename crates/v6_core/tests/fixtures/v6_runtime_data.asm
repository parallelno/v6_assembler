; Object-mode producer fixture for the 23 real runtime blocks studied by
; temp/pack/pack.py. Each block becomes one independently collectible ELF input
; section; ld.lld owns final placement. ROM mode still uses the local packer.

.global *

.pack window
hero_resources:
  .storage 17
.endpack

.pack
os_io_data:
  .storage 17
.endpack

.pack
switch_statuses:
  .storage 2
.endpack

.pack
global_states:
  .storage 10
.endpack

.pack
chars_runtime_data:
  .storage 482
.endpack

.pack window
overlays_runtime_data:
  .storage 227
.endpack

.pack
actor_data_head_ptr:
  .storage 2
.endpack

.pack
lv_data_init_tbl:
  .storage 14
.endpack

.pack
room_tiledata_backup:
  .storage 240
.endpack

.pack
temp_buff:
  .storage 512
.endpack

.pack
room_teleports_data:
  .storage 16
.endpack

.pack
game_status:
  .storage 16
.endpack

.pack
room_tiles_gfx_ptrs:
  .storage 480
.endpack

.pack align
room_tiledata:
  .storage 240
.endpack

.pack
palette:
  .storage 16
.endpack

.pack align
containers_inst_data_ptrs:
  .storage 256
.endpack

.pack align
resources_inst_data_ptrs:
  .storage 256
.endpack

.pack align
breakables_status:
  .storage 256
.endpack

.pack window
backs_runtime_data:
  .storage 62
.endpack

.pack
hero_runtime_data:
  .storage 31
.endpack

.pack window
rooms_spawn_rate:
  .storage 64
.endpack

.pack
global_items:
  .storage 15
.endpack

.pack
ram_disk_mode:
  .storage 1
.endpack
