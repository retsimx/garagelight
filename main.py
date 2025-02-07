import asyncio
import json

import machine
import dht

from ble import ble_task
from mqtt_as import MQTTClient, config
from secrets import WIFI_SSID, WIFI_PASSWORD, MQTT_IP


DHT_PIN = 16
LIGHT_PIN_NUM = 28

light_pin = machine.Pin(LIGHT_PIN_NUM, machine.Pin.OUT, value=1)

dht_sensor = dht.DHT11(machine.Pin(DHT_PIN))

# Local configuration
config["ssid"] = WIFI_SSID
config["wifi_pw"] = WIFI_PASSWORD
config["server"] = MQTT_IP


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
        try:
            dht_sensor.measure()

            payload = {
                "temp": dht_sensor.temperature(),
                "humidity": dht_sensor.humidity(),
            }

            print("Publishing:", payload)

            await client.publish("garage/temperature", json.dumps(payload), qos=1)
        except Exception as e:
            print("Error reading DHT sensor:", e)

        await asyncio.sleep(15)


config["queue_len"] = 6
MQTTClient.DEBUG = True
_client = MQTTClient(config)
try:
    asyncio.run(main(_client))
finally:
    _client.close()
