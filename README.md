# Pico W Rust Garage Beam
Raspberry Pi Pico W Rust UDP garage beam sensor for turning on/off a light via the garage light project.


To build the project, create a .env file with the wifi details used by the wifi driver. eg:
```
export WIFI_NETWORK=myssid
export WIFI_PASSWORD=mypassword
export WIFI_SEED=123456789ABCDEF0 # Must be a 64bit hex value without a 0x prefix
```

Then run `bash build.sh`.

Finally copy the uf2 file from the build directory to the pico w.
