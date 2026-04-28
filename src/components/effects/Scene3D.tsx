import { useEffect, useRef } from "react";
import * as THREE from "three";
import type { VpnStatus } from "../../stores/vpnStore";
import { useSettingsStore } from "../../stores/settingsStore";

/**
 * Three.js фон-сцена: octahedron-кристалл + wireframe + ядро + halo +
 * satellite shards + orbital rings + точечные «потоки шифра».
 *
 * Адаптировано из nemefisto.online (scene.js, sceneType=crystal):
 * - меньше частиц (250 вместо 800) — десктоп-окно компактное;
 * - нет starfield — фоновая `.grid-bg` уже даёт чувство глубины;
 * - нет tunnel/globe сцен — только crystal как символ VPN-щита.
 *
 * Реагирует на VPN-status:
 * - stopped: тусклый, медленное вращение;
 * - starting/stopping: ускоренное вращение, средний glow;
 * - running: яркий, пульсация ядра, шиммер;
 * - error: красно-оранжевый оттенок.
 *
 * Реактивность реализована через ref на target-state — animate() в каждом
 * кадре lerp-ит текущие значения к таргетным, поэтому смена статуса даёт
 * плавный переход без перезапуска сцены.
 */
export function Scene3D({ status }: { status: VpnStatus }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const theme = useSettingsStore((s) => s.theme);
  // target-state живёт в ref — animate() читает свежие значения без
  // пересоздания цикла. Меняется в useEffect ниже на изменениях
  // `status` или `theme`.
  const targetRef = useRef({
    glow: 0.4,
    speed: 0.5,
    tint: new THREE.Color(0xffffff),
    solidColor: new THREE.Color(0x0a0a0a),
    fogColor: new THREE.Color(0x050505),
  });

  // Меняем target при смене status или theme. animate() сам сгладит.
  useEffect(() => {
    const t = targetRef.current;

    // Палитра кристалла под каждую тему: tint линий, solid octahedron, fog.
    // baseTint используется для idle/active, dimTint для stopped (тусклее).
    type ThemePalette = {
      base: number;
      dim: number;
      solid: number;
      fog: number;
    };
    const palettes: Record<typeof theme, ThemePalette> = {
      dark:     { base: 0xffffff, dim: 0xbfbfbf, solid: 0x0a0a0a, fog: 0x050505 },
      light:    { base: 0x1c1b1a, dim: 0x6c6a66, solid: 0xf0eee8, fog: 0xf5f4ef },
      midnight: { base: 0x818cf8, dim: 0x5a64b8, solid: 0x14143a, fog: 0x0a0a18 },
      sunset:   { base: 0xff9b6e, dim: 0xc06d4a, solid: 0x2c1612, fog: 0x1a0d0a },
    };
    const pal = palettes[theme] ?? palettes.dark;

    switch (status) {
      case "stopped":
        t.glow = 0.35;
        t.speed = 0.4;
        t.tint.setHex(pal.dim);
        break;
      case "starting":
      case "stopping":
        t.glow = 0.65;
        t.speed = 1.4;
        t.tint.setHex(pal.base);
        break;
      case "running":
        t.glow = 0.9;
        t.speed = 1.0;
        t.tint.setHex(pal.base);
        break;
      case "error":
        // Warn-оранжевый одинаков во всех темах — это сигнал ошибки,
        // не подвластен brand-палитре.
        t.glow = 0.7;
        t.speed = 0.8;
        t.tint.setHex(0xd97757);
        break;
    }

    t.solidColor.setHex(pal.solid);
    t.fogColor.setHex(pal.fog);
  }, [status, theme]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const renderer = new THREE.WebGLRenderer({
      canvas,
      antialias: true,
      alpha: true,
      powerPreference: "high-performance",
    });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setSize(window.innerWidth, window.innerHeight);
    renderer.setClearColor(0x000000, 0);

    const scene = new THREE.Scene();
    const fog = new THREE.FogExp2(0x050505, 0.06);
    scene.fog = fog;

    const camera = new THREE.PerspectiveCamera(
      45,
      window.innerWidth / window.innerHeight,
      0.1,
      100
    );
    camera.position.set(0, 0, 8);

    // ── Свет ─────────────────────────────────────────────────────────────
    scene.add(new THREE.AmbientLight(0xffffff, 0.4));
    const keyLight = new THREE.DirectionalLight(0xffffff, 1.4);
    keyLight.position.set(4, 6, 6);
    scene.add(keyLight);
    const rimLight = new THREE.DirectionalLight(0xffffff, 1.0);
    rimLight.position.set(-5, -2, -3);
    scene.add(rimLight);
    const fillLight = new THREE.PointLight(0xffffff, 0.6, 20);
    fillLight.position.set(0, 0, 5);
    scene.add(fillLight);

    // ── Корневая группа ──────────────────────────────────────────────────
    const root = new THREE.Group();
    scene.add(root);

    // ── Crystal (octahedron solid + wireframe + inner + core) ────────────
    const crystalGroup = new THREE.Group();
    const geo = new THREE.OctahedronGeometry(1.6, 0);

    const matSolid = new THREE.MeshStandardMaterial({
      color: 0x0a0a0a,
      metalness: 0.9,
      roughness: 0.15,
      flatShading: true,
      emissive: 0x111111,
    });
    crystalGroup.add(new THREE.Mesh(geo, matSolid));

    const wireMat = new THREE.LineBasicMaterial({
      color: 0xffffff,
      transparent: true,
      opacity: 0.85,
    });
    crystalGroup.add(new THREE.LineSegments(new THREE.EdgesGeometry(geo), wireMat));

    const inner = new THREE.Mesh(
      new THREE.OctahedronGeometry(0.85, 0),
      new THREE.MeshBasicMaterial({
        color: 0xffffff,
        wireframe: true,
        transparent: true,
        opacity: 0.35,
      })
    );
    crystalGroup.add(inner);

    const core = new THREE.Mesh(
      new THREE.IcosahedronGeometry(0.28, 0),
      new THREE.MeshBasicMaterial({ color: 0xffffff })
    );
    crystalGroup.add(core);

    root.add(crystalGroup);

    // ── Satellite shards (5 штук орбитают вокруг crystal) ────────────────
    const shardGeos = [
      new THREE.TetrahedronGeometry(0.14, 0),
      new THREE.OctahedronGeometry(0.11, 0),
      new THREE.IcosahedronGeometry(0.1, 0),
    ];
    type ShardOrbit = {
      radius: number;
      speed: number;
      phase: number;
      tilt: number;
      ySpeed: number;
      yAmp: number;
      spinX: number;
      spinY: number;
      mat: THREE.LineBasicMaterial;
    };
    const shards: { mesh: THREE.LineSegments; orbit: ShardOrbit }[] = [];
    for (let i = 0; i < 5; i++) {
      const g = shardGeos[i % shardGeos.length];
      const mat = new THREE.LineBasicMaterial({
        color: 0xffffff,
        transparent: true,
        opacity: 0.65,
      });
      const mesh = new THREE.LineSegments(new THREE.EdgesGeometry(g), mat);
      const orbit: ShardOrbit = {
        radius: 2.8 + Math.random() * 1.2,
        speed: 0.15 + Math.random() * 0.25,
        phase: Math.random() * Math.PI * 2,
        tilt: (Math.random() - 0.5) * 1.4,
        ySpeed: 0.5 + Math.random() * 0.8,
        yAmp: 0.3 + Math.random() * 0.4,
        spinX: (Math.random() - 0.5) * 1.5,
        spinY: (Math.random() - 0.5) * 1.5,
        mat,
      };
      crystalGroup.add(mesh);
      shards.push({ mesh, orbit });
    }

    // ── Orbital rings (3 тонких тора вокруг crystal) ─────────────────────
    const ringsGroup = new THREE.Group();
    const rings: THREE.Mesh[] = [];
    for (let i = 0; i < 3; i++) {
      const r = 2.4 + i * 0.5;
      const ring = new THREE.Mesh(
        new THREE.TorusGeometry(r, 0.005, 8, 200),
        new THREE.MeshBasicMaterial({
          color: 0xffffff,
          transparent: true,
          opacity: 0.18 - i * 0.04,
        })
      );
      ring.rotation.x = Math.PI / 2 + i * 0.4;
      ring.rotation.y = i * 0.3;
      ringsGroup.add(ring);
      rings.push(ring);
    }
    root.add(ringsGroup);

    // ── Encrypted streams (точки летят к центру) ─────────────────────────
    const STREAM_COUNT = 250;
    const streamGeo = new THREE.BufferGeometry();
    const streamPositions = new Float32Array(STREAM_COUNT * 3);
    type StreamData = { radius: number; theta: number; phi: number; speed: number };
    const streamData: StreamData[] = [];
    for (let i = 0; i < STREAM_COUNT; i++) {
      const radius = 3 + Math.random() * 6;
      const theta = Math.random() * Math.PI * 2;
      const phi = Math.acos(2 * Math.random() - 1);
      streamData.push({
        radius,
        theta,
        phi,
        speed: 0.005 + Math.random() * 0.02,
      });
      streamPositions[i * 3] = radius * Math.sin(phi) * Math.cos(theta);
      streamPositions[i * 3 + 1] = radius * Math.sin(phi) * Math.sin(theta);
      streamPositions[i * 3 + 2] = radius * Math.cos(phi);
    }
    streamGeo.setAttribute(
      "position",
      new THREE.BufferAttribute(streamPositions, 3)
    );
    const streamMat = new THREE.PointsMaterial({
      color: 0xffffff,
      size: 0.022,
      transparent: true,
      opacity: 0.6,
      sizeAttenuation: true,
      depthWrite: false,
    });
    const streams = new THREE.Points(streamGeo, streamMat);
    root.add(streams);

    // ── Mouse parallax ───────────────────────────────────────────────────
    const mouse = { x: 0, y: 0 };
    const mouseTarget = { x: 0, y: 0 };
    const onMove = (e: MouseEvent) => {
      mouseTarget.x = (e.clientX / window.innerWidth) * 2 - 1;
      mouseTarget.y = -((e.clientY / window.innerHeight) * 2 - 1);
    };
    window.addEventListener("mousemove", onMove);

    // ── Resize ───────────────────────────────────────────────────────────
    const onResize = () => {
      camera.aspect = window.innerWidth / window.innerHeight;
      camera.updateProjectionMatrix();
      renderer.setSize(window.innerWidth, window.innerHeight);
    };
    window.addEventListener("resize", onResize);

    // ── Pause when window hidden (экономия GPU/батареи) ──────────────────
    let visible = !document.hidden;
    const onVisibility = () => {
      visible = !document.hidden;
    };
    document.addEventListener("visibilitychange", onVisibility);

    // ── Animation loop ───────────────────────────────────────────────────
    const clock = new THREE.Clock();
    let raf = 0;
    // Текущие значения lerp-ятся к target (плавно).
    let glow = 0.4;
    let speed = 0.5;
    const tint = new THREE.Color(0xffffff);
    let time = 0;

    const animate = () => {
      raf = requestAnimationFrame(animate);
      if (!visible) return;
      const dt = clock.getDelta();

      // Smooth interpolation: glow/speed/tint/тема → target
      const t = targetRef.current;
      glow += (t.glow - glow) * 0.05;
      speed += (t.speed - speed) * 0.05;
      tint.lerp(t.tint, 0.05);
      // Solid octahedron + fog плавно подтягиваются к нужной теме
      (matSolid.color as THREE.Color).lerp(t.solidColor, 0.05);
      fog.color.lerp(t.fogColor, 0.05);

      time += dt * speed;
      const tm = time;

      // Mouse smoothing
      mouse.x += (mouseTarget.x - mouse.x) * 0.05;
      mouse.y += (mouseTarget.y - mouse.y) * 0.05;

      // Apply tint to all white materials
      wireMat.color.copy(tint);
      streamMat.color.copy(tint);
      shards.forEach(({ orbit }) => orbit.mat.color.copy(tint));
      rings.forEach((r) => (r.material as THREE.MeshBasicMaterial).color.copy(tint));

      // Glow → opacity
      streamMat.opacity = 0.3 + glow * 0.7;
      wireMat.opacity = 0.4 + glow * 0.5;

      // Root rotation: idle + parallax
      root.rotation.y = tm * 0.18 + mouse.x * 0.35;
      root.rotation.x = Math.sin(tm * 0.3) * 0.1 + mouse.y * 0.25;

      // Crystal inner spin + core pulse
      inner.rotation.y -= dt * 0.6 * speed;
      inner.rotation.x += dt * 0.3 * speed;
      core.rotation.y += dt * speed;
      const pulse = 1 + Math.sin(tm * 2) * 0.08;
      core.scale.setScalar(pulse * (0.7 + glow * 0.5));

      // Rings spin
      rings.forEach((ring, i) => {
        ring.rotation.z += dt * (0.1 + i * 0.05) * speed;
        ring.rotation.x += dt * 0.05 * speed;
      });

      // Shards orbit
      shards.forEach(({ mesh, orbit }) => {
        const a = tm * orbit.speed + orbit.phase;
        mesh.position.x = Math.cos(a) * orbit.radius;
        mesh.position.z = Math.sin(a) * orbit.radius;
        mesh.position.y = Math.sin(tm * orbit.ySpeed + orbit.phase) * orbit.yAmp;
        mesh.rotation.x += dt * orbit.spinX * speed;
        mesh.rotation.y += dt * orbit.spinY * speed;
        mesh.rotation.z = orbit.tilt;
        orbit.mat.opacity = 0.4 + glow * 0.5;
      });

      // Streams flow inward
      const positions = streams.geometry.attributes.position
        .array as Float32Array;
      for (let i = 0; i < STREAM_COUNT; i++) {
        const d = streamData[i];
        d.radius -= d.speed * speed * 60 * dt;
        if (d.radius < 1.8) {
          d.radius = 7 + Math.random() * 3;
          d.theta = Math.random() * Math.PI * 2;
          d.phi = Math.acos(2 * Math.random() - 1);
        }
        positions[i * 3] = d.radius * Math.sin(d.phi) * Math.cos(d.theta);
        positions[i * 3 + 1] = d.radius * Math.sin(d.phi) * Math.sin(d.theta);
        positions[i * 3 + 2] = d.radius * Math.cos(d.phi);
      }
      streams.geometry.attributes.position.needsUpdate = true;

      renderer.render(scene, camera);
    };
    animate();

    // ── Cleanup при размонтировании ──────────────────────────────────────
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("resize", onResize);
      document.removeEventListener("visibilitychange", onVisibility);

      // dispose всех geometry/material — важно при HMR в dev,
      // иначе утекают GPU-buffers.
      geo.dispose();
      matSolid.dispose();
      wireMat.dispose();
      inner.geometry.dispose();
      (inner.material as THREE.Material).dispose();
      core.geometry.dispose();
      (core.material as THREE.Material).dispose();
      streamGeo.dispose();
      streamMat.dispose();
      shards.forEach(({ mesh, orbit }) => {
        mesh.geometry.dispose();
        orbit.mat.dispose();
      });
      rings.forEach((r) => {
        r.geometry.dispose();
        (r.material as THREE.Material).dispose();
      });
      shardGeos.forEach((g) => g.dispose());
      renderer.dispose();
    };
  }, []);

  return <canvas ref={canvasRef} className="scene-canvas" />;
}
