# Hardware & Software

Some information about the hardware and software I currently use:

# Hardware

- [RaspberryPi 4b](https://www.raspberrypi.org/products/raspberry-pi-4-model-b/)
- [Neuftech USB RFID Reader](https://www.amazon.de/Neuftech-Reader-Kartenleseger%C3%A4t-Kartenleser-Kontaktlos/dp/B018OYOR3E)
- [Pirate Audio: 3W Stereo Amp for Raspberry Pi](Pirate Audio: 3W Stereo Amp for Raspberry Pi)

# Software

For the Pirate Audio, I need to modify `/boot/config.txt` to disable built in audio (I don't want to use that)
and to enable the Pirate Audio hat:

```
# this is part of /boot/config.txt

# Enable audio (loads snd_bcm2835)
dtparam=audio=off

# for the audio hat
dtoverlay=hifiberry-dac
gpio=25=op,dh
```

There is no volume control for the audio device, so I'm creating a soft control with
a custom `~/.asoundrc` file:

```
pcm.softvol {
    type            softvol
    slave {
        pcm         "cards.pcm.default"
    }
    control {
        name        "SoftMaster"
        card        0
    }
}

pcm.!default {
    type             plug
    slave.pcm       "softvol"
}
```
