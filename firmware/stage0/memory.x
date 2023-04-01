MEMORY
{
  /* NOTE 1 K = 1 KiBi = 1024 bytes */
  FLASH : ORIGIN = 0x00000000, LENGTH = 32K
  FLASH_UNUSED : ORIGIN = 0x00008000, LENGTH = (1024K - LENGTH(FLASH))

  SCRATCH: ORIGIN = 0x20000000, LENGTH = 224K
  RAM : ORIGIN = 0x20038000, LENGTH = (32K - 64)
  MAGIC: ORIGIN = 0x2003FFC0, LENGTH = 64
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


/* Do not exceed this mark in the error messages below                                    | */
ASSERT(LENGTH(SCRATCH) + LENGTH(MAGIC) + LENGTH(RAM) <= 256K, "
ERROR(stage0): Total RAM size is too big? Check you haven't added new sections!");
