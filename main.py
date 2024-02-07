import asyncio

import machine

from ble import ble_task
from mqtt_as import MQTTClient, config
from secrets import WIFI_SSID, WIFI_PASSWORD, MQTT_IP

LIGHT_PIN_NUM = 28
light_pin = machine.Pin(LIGHT_PIN_NUM, machine.Pin.OUT, value=1)

# Local configuration
config['ssid'] = WIFI_SSID
config['wifi_pw'] = WIFI_PASSWORD
config['server'] = MQTT_IP


async def messages(client):
    async for topic, msg, retained in client.queue:
        if topic.startswith("garagelight/reset"):
            machine.reset()

        else:
            print("Unknown MQTT message:", topic, msg, retained)


async def up(client):
    while True:
        await client.up.wait()
        client.up.clear()
        await client.subscribe("garagelight/reset", 0)


async def main(client):
    asyncio.create_task(ble_task(light_pin))

    await client.connect()
    for coroutine in (up, messages):
        asyncio.create_task(coroutine(client))

    while True:
        await asyncio.sleep(5)


config["queue_len"] = 6
MQTTClient.DEBUG = True
_client = MQTTClient(config)
try:
    asyncio.run(main(_client))
finally:
    _client.close()
