MEMORY
{
  /* NOTE 1 K = 1 KiBi = 1024 bytes */
  FLASH : ORIGIN = 0x00000000, LENGTH = 1024K
  SCRATCH: ORIGIN = 0x20000000, LENGTH = 224K
  MAGIC: ORIGIN = 0x20038000, LENGTH = 8
  RAM : ORIGIN = 0x20038008, LENGTH = (32K - 8)
}

SECTIONS
{
    .magic (NOLOAD) : ALIGN(8)
    {
        *(.magic .magic.*);
        KEEP(*(.magic .magic.*));
        . = ALIGN(8);
    } > MAGIC

    .scratch (NOLOAD) : ALIGN(8)
    {
        *(.scratch .scratch.*);
        KEEP(*(.scratch .scratch.*));
        . = ALIGN(8);
    } > SCRATCH
}
