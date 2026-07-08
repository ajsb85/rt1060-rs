    .section .payload, "a"
    .global _payload_start
    .global _payload_end
    .balign 4
_payload_start:
    .incbin "swiftio.bin"
    .balign 4
_payload_end:
