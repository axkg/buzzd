# buzzd

`buzzd` listens for commands sent via MQTT to play various configurable beeping patterns using an active buzzer connected to a GPIO pin of a *Raspberry Pi*.

This tool is a rewrite of an unreleased C based version I have been using for quite some time. As the legacy version was lacking flexibility, I rewrote it in Rust to learn something new.

## Prerequisites

### Active Buzzer

The active buzzer I use buzzes when the GPIO pin is low and is silent if the pin is high. In case other buzzers behave differently, an additional configuration option to invert the pin use might be required.

### Real-time Scheduling

`buzzd` will try to set real-time scheduling policies fo the *pattern playback thread* to minimize the impact on the playback rhythm caused by other processes. If you are not running as root, add a line to `/etc/security/limits.conf` - replacing the `user` with the actual username:

```
user           -       rtprio         99
```
Where user is replaced with the username of the user running `buzzd`.

## Configuration

On startup `buzzd` will try to read it's configuration file `buzzd.json`. It will try to find it in:
* the current directory
* the `.config` directory in the executing user's home
* in `/etc`

### JSON structure

Please refer to the [example configuration file](buzzd.json) that comes with `buzzd` to adapt to your use case. The following parameters can be set at top level:

* `gpio`: The GPIO pin the active buzzer is connected to (integer)
* `pause`: The pause that will be introduced between repeated patterns in milliseconds (integer)

The connection to the MQTT broker can be setup in the `mqtt` section:

* `broker`: IP address or server name of the MQTT broker, `localhost` by default - according to the paho-mqtt documentation URIs should work, too (e.g. `mqtt://server:port`) - but that does not work for me currently
* `topic`: The MQTT topic `buzzd` should react upon, `buzzd` by default

### Patterns

The next section in the configuration file defines a list of beeping `patterns`. A pattern may look like the following example - note that the comments prefixed with `//` are here for documentation purposes only, they cannot be included in the actual configuration file as they violate the JSON syntax.

With regards to the pattern name, the name `'_'` (just the underscore) is a reserved name. It can be used to cancel the playback of the current pattern without triggering a new one.

```json
{
    "name": "ack", // the name of the pattern
    "repeat": 3,   // repeat 3 times by default
    "rhythm": [
        75,        // buzz for 75 milliseconds
        75,        // pause for 75 milliseconds
        75         // buzz for 75 milliseconds
    ]
}
```
## MQTT Commands

To trigger the playback of a configured pattern, publish the pattern name with the configured topic:
```bash
mosquitto_pub -h localhost -t actors/buzzer -m "ack"
```
Provide an additional number of repetitions to override the default value configured with the pattern:
```bash
mosquitto_pub -h localhost -t actors/buzzer -m "ack 2"
```
As noted above, it is possible to cancel any pattern playback currently in process triggering the playback of the reserved *cancel* pattern:
```bash
mosquitto_pub -h localhost -t actors/buzzer -m "_"
```
