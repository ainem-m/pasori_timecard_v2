#!/usr/bin/env python3
"""RC-S380 USB 診断スクリプト - nfcpy と同じことを Python で試す"""
import usb.core
import usb.util
import time

VID, PID = 0x054C, 0x06C3

def frame(data):
    """拡張フレーム構築 (nfcpy Frame クラスと同じロジック)"""
    import struct
    f = bytearray([0, 0, 255, 255, 255])
    f += bytearray(struct.pack("<H", len(data)))
    f += bytearray([(256 - sum(f[5:7])) % 256])
    f += bytearray(data)
    f += bytearray([(256 - sum(f[8:])) % 256, 0])
    return bytes(f)

def send_recv(ep_out, ep_in, name, data):
    cmd = frame(data)
    print(f"\n[{name}] 送信: {cmd.hex(' ').upper()}")
    ep_out.write(cmd)
    try:
        r1 = ep_in.read(256, timeout=1000)
        print(f"  受信1 ({len(r1)} bytes): {bytes(r1).hex(' ').upper()}")
    except usb.core.USBTimeoutError:
        print("  受信1: Timeout")
        return
    try:
        r2 = ep_in.read(256, timeout=1000)
        print(f"  受信2 ({len(r2)} bytes): {bytes(r2).hex(' ').upper()}")
    except usb.core.USBTimeoutError:
        print("  受信2: Timeout")

dev = usb.core.find(idVendor=VID, idProduct=PID)
if dev is None:
    print("RC-S380 が見つかりません")
    exit(1)

print(f"RC-S380 発見: Bus {dev.bus:03d} Device {dev.address:03d}")
print(f"現在の設定: {dev.get_active_configuration().bConfigurationValue}")

# PyUSB が自動でやること: set_configuration
print("\n--- set_configuration(1) ---")
dev.set_configuration(1)
print("OK")

intf = dev.get_active_configuration()[(0, 0)]
ep_out = usb.util.find_descriptor(intf,
    custom_match=lambda e: usb.util.endpoint_direction(e.bEndpointAddress) == usb.util.ENDPOINT_OUT)
ep_in = usb.util.find_descriptor(intf,
    custom_match=lambda e: usb.util.endpoint_direction(e.bEndpointAddress) == usb.util.ENDPOINT_IN)

print(f"EP OUT: 0x{ep_out.bEndpointAddress:02X}, EP IN: 0x{ep_in.bEndpointAddress:02X}")

# ACK ソフトリセット
print("\n--- ACK ソフトリセット ---")
ACK = bytes([0x00, 0x00, 0xFF, 0x00, 0xFF, 0x00])
ep_out.write(ACK)
time.sleep(0.01)
# ドレイン
drained = 0
while True:
    try:
        data = ep_in.read(256, timeout=50)
        print(f"  ドレイン: {bytes(data).hex(' ').upper()}")
        drained += 1
    except usb.core.USBTimeoutError:
        break
print(f"ドレイン完了 ({drained} packets)")

# コマンド試行
send_recv(ep_out, ep_in, "SetCommandType(1)", [0xD6, 0x2A, 0x01])
send_recv(ep_out, ep_in, "GetFirmwareVersion", [0xD6, 0x20])
send_recv(ep_out, ep_in, "SwitchRF(off)",      [0xD6, 0x06, 0x00])
