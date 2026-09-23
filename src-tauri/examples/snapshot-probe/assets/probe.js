/* Development fixture only; no production UI imports or database access. */
const invoke = window.__TAURI__.core.invoke;
const palettes = [
  ['#f7f9fc', '#e9eef5', '#223044', '#365ec7'],
  ['#302b22', '#242119', '#e9e0cf', '#e4b379'],
  ['#20272b', '#191e21', '#e0e8e8', '#a5cfca'],
];
const frames = [];
async function paint() {
  await document.fonts.ready;
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
}
(async () => {
  try {
    for (let i = 0; i < 100; i++) {
      const revision = await invoke('begin_frame');
      const palette = palettes[i % palettes.length];
      ['--canvas', '--panel', '--ink', '--accent'].forEach((key, n) => document.documentElement.style.setProperty(key, palette[n]));
      const marker = [revision % 256, (revision * 37) % 256, (revision * 83) % 256];
      document.getElementById('marker').style.background = `rgb(${marker.join(',')})`;
      document.getElementById('revision').textContent = `Revision ${revision}`;
      await paint();
      const frame = await invoke('capture_ready', { revision });
      frames.push({revision, marker, canvas:palette[0],
        viewport:[window.innerWidth, window.innerHeight], device_pixel_ratio:window.devicePixelRatio, ...frame});
    }
    const checks = await invoke('exercise_limits');
    await invoke('finish_probe', { details: {frames, checks} });
  } catch (error) {
    await invoke('finish_probe', {details: {error:String(error), frames}});
  }
})();
