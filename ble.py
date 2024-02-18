import sys

from micropython import const

sys.path.append("")

import uasyncio as asyncio
import aioble
import bluetooth

# org.bluetooth.service.environmental_sensing
_ENV_SENSE_UUID = bluetooth.UUID(0x181A)
# org.bluetooth.characteristic.temperature
_ENV_SENSE_TEMP_UUID = bluetooth.UUID(0x2A6E)
# org.bluetooth.characteristic.gap.appearance.xml
_ADV_APPEARANCE_GENERIC_THERMOMETER = const(768)

# How frequently to send advertising beacons.
_ADV_INTERVAL_MS = 250_000


# Register GATT server.
temp_service = aioble.Service(_ENV_SENSE_UUID)
temp_characteristic = aioble.Characteristic(
    temp_service, _ENV_SENSE_TEMP_UUID, write=True, notify=True
)
aioble.register_services(temp_service)


async def control_task(connection, light_pin):
    global send_file, recv_file, list_path

    try:
        with connection.timeout(None):
            while True:
                print("Waiting for write")
                await temp_characteristic.written()
                msg = temp_characteristic.read()

                if msg[0]:
                    light_pin.off()
                else:
                    light_pin.on()

    except aioble.DeviceDisconnectedError:
        return


async def ble_task(light_pin):
    while True:
        try:
            async with await aioble.advertise(
                _ADV_INTERVAL_MS,
                name="glgt",
                services=[_ENV_SENSE_UUID],
                appearance=_ADV_APPEARANCE_GENERIC_THERMOMETER,
            ) as connection:
                print("Connection from", connection.device)

                await control_task(connection, light_pin)

                await connection.disconnected()
        except:
            await asyncio.sleep_ms(1000)
