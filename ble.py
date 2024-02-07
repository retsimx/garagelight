import sys

sys.path.append("")

import uasyncio as asyncio
import aioble
import bluetooth

import struct

# org.bluetooth.service.environmental_sensing
_ENV_SENSE_UUID = bluetooth.UUID(0x181A)
# org.bluetooth.characteristic.temperature
_ENV_SENSE_TEMP_UUID = bluetooth.UUID(0x2A6E)


# Helper to decode the temperature characteristic encoding (sint16, hundredths of a degree).
def _decode_temperature(data):
    return struct.unpack("<h", data)[0]


async def find_temp_sensor():
    # Scan for 5 seconds, in active mode, with very low interval/window (to
    # maximise detection rate).
    async with aioble.scan(5000, interval_us=30000, window_us=30000, active=True) as scanner:
        async for result in scanner:
            # See if it matches our name and the environmental sensing service.
            if result.name() == "mpy-temp" and _ENV_SENSE_UUID in result.services():
                return result.device
    return None


async def ble_task(light_pin):
    while True:
        await asyncio.sleep_ms(1000)

        try:
            device = await find_temp_sensor()
            if not device:
                print("Temperature sensor not found")
                continue

            try:
                print("Connecting to", device)
                connection = await device.connect()
            except asyncio.TimeoutError:
                print("Timeout during connection")
                continue

            async with connection:
                try:
                    temp_service = await connection.service(_ENV_SENSE_UUID)
                    temp_characteristic = await temp_service.characteristic(_ENV_SENSE_TEMP_UUID)
                except asyncio.TimeoutError:
                    print("Timeout discovering services/characteristics")
                    continue

                while True:
                    beam_state = await temp_characteristic.read()
                    if beam_state[0]:
                        light_pin.off()
                    else:
                        light_pin.on()

                    await asyncio.sleep_ms(5)
        except Exception as e:
            print(e)
            pass
