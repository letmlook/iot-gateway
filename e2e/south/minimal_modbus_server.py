#!/usr/bin/env python3
"""Minimal Modbus TCP server using asyncio standard library - no pymodbus dependency needed."""
import asyncio

MB_FUNCTION_READ_HOLDING = 0x03
MB_FUNCTION_READ_INPUT   = 0x04

class ModbusSession(asyncio.Protocol):
    """One session per TCP connection."""

    def __init__(self):
        # holding registers 1-6 with values [100, 200, 300, 400, 500, 600]
        self.hr = {i: 100 * i for i in range(1, 7)}
        self.transport = None

    def connection_made(self, transport):
        self.transport = transport

    def data_received(self, data):
        """Parse Modbus TCP request and send response."""
        if len(data) < 8:
            return

        # Modbus TCP ADU:
        # 0-1: Transaction ID (we echo it back)
        # 2-3: Protocol ID (0 = Modbus)
        # 4-5: Length (byte count after this field)
        # 6: Unit ID
        # 7: Function code
        # 8+: Request data
        tid  = data[0] << 8 | data[1]
        proto = data[2] << 8 | data[3]
        length = data[4] << 8 | data[5]
        unit   = data[6]
        func   = data[7]

        if func == MB_FUNCTION_READ_HOLDING:
            start = data[8] << 8 | data[9]
            count = data[10] << 8 | data[11]
            values = [self.hr.get(start + i + 1, 0) for i in range(count)]  # 1-indexed
            byte_count = count * 2
            resp = bytearray([
                (tid >> 8) & 0xFF, tid & 0xFF,
                (proto >> 8) & 0xFF, proto & 0xFF,
                0, 3 + byte_count,  # length
                unit, func, byte_count,
            ])
            for v in values:
                resp.append((v >> 8) & 0xFF)
                resp.append(v & 0xFF)
            self.transport.write(bytes(resp))

        elif func == MB_FUNCTION_READ_INPUT:
            start = data[8] << 8 | data[9]
            count = data[10] << 8 | data[11]
            values = [0] * count
            byte_count = count * 2
            resp = bytearray([
                (tid >> 8) & 0xFF, tid & 0xFF,
                (proto >> 8) & 0xFF, proto & 0xFF,
                0, 3 + byte_count,
                unit, func, byte_count,
            ])
            for v in values:
                resp.append((v >> 8) & 0xFF)
                resp.append(v & 0xFF)
            self.transport.write(bytes(resp))

        else:
            # Function not supported - echo with error bit set
            resp = bytearray([data[0], data[1], 0, 0, 0, 3, unit, func | 0x80, 0x01])
            self.transport.write(bytes(resp))

    def connection_lost(self, exc):
        pass


async def main():
    loop = asyncio.get_running_loop()
    server = await loop.create_server(lambda: ModbusSession(), '127.0.0.1', 10502)
    print("Minimal Modbus TCP server listening on 127.0.0.1:10502")
    print("Holding registers 1-6: [100, 200, 300, 400, 500, 600]")
    await asyncio.Event().wait()


if __name__ == '__main__':
    asyncio.run(main())