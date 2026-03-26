; test program to set the palette.
; using direct port I/O. The palette data is defined at the end of the file.
			OPCODE_EI = 0xFB
			OPCODE_RET = 0xC9

.org 0x100
start:
			; Initialize registers to zero
			lxi sp, 0x8000
			lxi b, 0
			lxi d, 0
			lxi h, 0
			push b
			pop psw
			mvi a, OPCODE_EI
			sta 0x38
			mvi a, OPCODE_RET
			sta 0x39
			ei

	  		hlt
	  		call set_palette
end:
	  		di
			; end of program
	  		hlt

PALETTE_LEN = 16
set_palette: ; non-local label
			lxi h, palette + PALETTE_LEN - 1
			mvi	a, 0x88
			out	0
			mvi	b, 0x0F

@loop: ; local label. can be reused if separated by non-local label
			mov	a, b
			out	2
			mov a, m
			out 0x0C
			push psw
			pop psw
			push psw
			pop psw
			dcx h
			dcr b
			out 0x0C

			jp	@loop
			ei
			ret
palette:
	  		DB b11_111_000, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, b00_000_000,
	  		DB 0x20, 0x21, 0x22, 0x23, 0x24, 0x25, 0x26, 0x27,
