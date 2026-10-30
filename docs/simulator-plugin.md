# Video simulator plugin

The simulator plugin exposes one software device with serial
`EventSimulator`. Its creation schema contains:

- `video_file` — required video input;
- `fps` — optional positive FPS override;
- `width` — optional output width, default 800;
- `height` — optional output height, default 600;
- `preload` — optional flag that decodes the whole video up front, default
  `false`;
- `config_file` — optional TOML file containing the initial `EvsParameters`.

If `fps` is omitted, the plugin uses the video's encoded average frame rate.
If `config_file` is omitted, the plugin uses validated simulator defaults.

The simulator advertises separate `Index` and `Seek` facilities. `Index`
reports `t_min = 0` and the video stream duration in microseconds. `Seek`
replays frames from the beginning up to the requested timestamp so the
decoder frame position and the stateful event model remain aligned. Seeking
also clears prefetched batches and resets the circuit/noise state.

After the application creates the device through the host layer, it subscribes
to CD events and starts the raw-event stream. Each stream request decodes one
video frame, converts RGB pixels to luminance photocurrents, assigns a
timestamp in microseconds from the frame index and FPS, and passes the frame
to the stateful `EvsSimulator`. The simulator holds each frame for its
video-frame duration and integrates it at the configured Forward-Euler time
step, so the model does not silently change behavior when the video FPS
changes. The plugin delivers generated events, including empty event batches, in
one callback to every subscribed CD sink. The plugin reuses its RGB and
photocurrent buffers across frames and owns the FFmpeg input, decoder, scaler,
and simulator state for the device lifetime.

The optional parameter file must contain the fields of `EvsParameters`, for
example:

```toml
a = 1.0
c_c = 1.0
c_lsf = 1.0
zeta = 1.2
v_t = 0.02585
i_d1_t0 = 0.1
photocurrent_scale = 1.0
i_d2_t0 = 1.0
i_sf = 1.0
dt = 0.01
tau_fe = 1.0
tau_o1 = 1.0
threshold_on = 0.001
threshold_off = 0.001

# Optional paper-model extensions; omitted values use a small noise floor and no dark current.
dark_current = 0.0
noise_fe_std = 0.0001
noise_o1_std = 0.0001
```

`noise_fe_std` and `noise_o1_std` are stationary voltage-noise standard
deviations. The simulator derives the white-noise input variance using Eq. 23
and applies it through the first-order autoregressive model in Eq. 19. Noise
uses a deterministic seed so runs are reproducible.

The host layer validates the creation schema before the configuration crosses
the ABI, and the plugin validates it again before opening the video or
parameter file.
