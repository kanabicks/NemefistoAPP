import { useEffect, useRef } from "react";
import * as THREE from "three";
import type { VpnStatus } from "../../stores/vpnStore";
import {
  PRESET_BACKGROUND,
  PRESET_SCENE_PALETTE,
  useSettingsStore,
} from "../../stores/settingsStore";

/**
 * Three.js фон с переключаемыми сценами.
 *
 * Все 4 варианта живут в одной WebGL-сцене и переключаются через
 * `visible = true/false` (см. `applySceneVisibility`). Объекты создаются
 * один раз при mount, animate() читает свежий target из ref'a и плавно
 * подтягивает glow/speed/tint/палитру темы — без полного ребуилда сцены
 * при смене статуса/темы/типа фона.
 *
 * Сцены:
 *  - crystal — octahedron-кристалл + ядро + 5 shards + 3 rings + streams;
 *  - tunnel — анимированный wireframe-torus knot;
 *  - globe — wireframe-сфера с точками-странами по lat/lon;
 *  - particles — поле точек с parallax от мыши, без вращающегося объекта.
 */
export function Scene3D({ status }: { status: VpnStatus }) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const theme = useSettingsStore((s) => s.theme);
  const background = useSettingsStore((s) => s.background);
  const preset = useSettingsStore((s) => s.preset);

  // Effective значения зависят от preset: если активен — берём из него,
  // иначе — из обычных настроек темы/фона.
  const effectiveBackground = preset === "none" ? background : PRESET_BACKGROUND[preset];

  const targetRef = useRef({
    glow: 0.4,
    speed: 0.5,
    tint: new THREE.Color(0xffffff),
    solidColor: new THREE.Color(0x0a0a0a),
    fogColor: new THREE.Color(0x050505),
    background: "crystal" as typeof background,
  });

  // Меняем target при смене status, theme, preset или background.
  useEffect(() => {
    const t = targetRef.current;
    type ScenePalette = {
      base: number;
      dim: number;
      solid: number;
      fog: number;
    };
    const themePalettes: Record<typeof theme, ScenePalette> = {
      dark:     { base: 0xffffff, dim: 0xbfbfbf, solid: 0x0a0a0a, fog: 0x050505 },
      light:    { base: 0x1c1b1a, dim: 0x6c6a66, solid: 0xf0eee8, fog: 0xf5f4ef },
      midnight: { base: 0x818cf8, dim: 0x5a64b8, solid: 0x14143a, fog: 0x0a0a18 },
      sunset:   { base: 0xff9b6e, dim: 0xc06d4a, solid: 0x2c1612, fog: 0x1a0d0a },
      sand:     { base: 0x6e4a25, dim: 0x9c7a55, solid: 0xc8a878, fog: 0xd8c2a0 },
    };
    // Если активен пресет — он переопределяет theme-палитру.
    const pal = preset === "none"
      ? (themePalettes[theme] ?? themePalettes.dark)
      : PRESET_SCENE_PALETTE[preset];

    switch (status) {
      case "stopped":
        t.glow = 0.35; t.speed = 0.4; t.tint.setHex(pal.dim); break;
      case "starting":
      case "stopping":
        t.glow = 0.65; t.speed = 1.4; t.tint.setHex(pal.base); break;
      case "running":
        t.glow = 0.9; t.speed = 1.0; t.tint.setHex(pal.base); break;
      case "error":
        t.glow = 0.7; t.speed = 0.8; t.tint.setHex(0xd97757); break;
    }
    t.solidColor.setHex(pal.solid);
    t.fogColor.setHex(pal.fog);
    t.background = effectiveBackground;
  }, [status, theme, background, preset, effectiveBackground]);

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
      100,
    );
    camera.position.set(0, 0, 8);

    // Свет
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

    // Корневая группа — крутится для crystal/tunnel/globe; для particles
    // вращение отключено (parallax даёт другой характер).
    const root = new THREE.Group();
    scene.add(root);

    // ════════════════════════════════════════════════════════════════════
    //  CRYSTAL
    // ════════════════════════════════════════════════════════════════════
    const crystalRoot = new THREE.Group();
    const crystalGeo = new THREE.OctahedronGeometry(1.6, 0);
    const matSolid = new THREE.MeshStandardMaterial({
      color: 0x0a0a0a,
      metalness: 0.9,
      roughness: 0.15,
      flatShading: true,
      emissive: 0x111111,
    });
    crystalRoot.add(new THREE.Mesh(crystalGeo, matSolid));

    const wireMat = new THREE.LineBasicMaterial({
      color: 0xffffff, transparent: true, opacity: 0.85,
    });
    crystalRoot.add(new THREE.LineSegments(new THREE.EdgesGeometry(crystalGeo), wireMat));

    const inner = new THREE.Mesh(
      new THREE.OctahedronGeometry(0.85, 0),
      new THREE.MeshBasicMaterial({ color: 0xffffff, wireframe: true, transparent: true, opacity: 0.35 }),
    );
    crystalRoot.add(inner);

    const core = new THREE.Mesh(
      new THREE.IcosahedronGeometry(0.28, 0),
      new THREE.MeshBasicMaterial({ color: 0xffffff }),
    );
    crystalRoot.add(core);

    // Shards (5 satellite octahedrons)
    const shardGeos = [
      new THREE.TetrahedronGeometry(0.14, 0),
      new THREE.OctahedronGeometry(0.11, 0),
      new THREE.IcosahedronGeometry(0.1, 0),
    ];
    type ShardOrbit = {
      radius: number; speed: number; phase: number; tilt: number;
      ySpeed: number; yAmp: number; spinX: number; spinY: number;
      mat: THREE.LineBasicMaterial;
    };
    const shards: { mesh: THREE.LineSegments; orbit: ShardOrbit }[] = [];
    for (let i = 0; i < 5; i++) {
      const g = shardGeos[i % shardGeos.length];
      const mat = new THREE.LineBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.65 });
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
      crystalRoot.add(mesh);
      shards.push({ mesh, orbit });
    }

    // Rings
    const rings: THREE.Mesh[] = [];
    for (let i = 0; i < 3; i++) {
      const r = 2.4 + i * 0.5;
      const ring = new THREE.Mesh(
        new THREE.TorusGeometry(r, 0.005, 8, 200),
        new THREE.MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.18 - i * 0.04 }),
      );
      ring.rotation.x = Math.PI / 2 + i * 0.4;
      ring.rotation.y = i * 0.3;
      crystalRoot.add(ring);
      rings.push(ring);
    }

    // Streams (точки летят к центру) — общие для crystal/tunnel/globe.
    const STREAM_COUNT = 250;
    const streamGeo = new THREE.BufferGeometry();
    const streamPositions = new Float32Array(STREAM_COUNT * 3);
    type StreamData = { radius: number; theta: number; phi: number; speed: number };
    const streamData: StreamData[] = [];
    for (let i = 0; i < STREAM_COUNT; i++) {
      const radius = 3 + Math.random() * 6;
      const theta = Math.random() * Math.PI * 2;
      const phi = Math.acos(2 * Math.random() - 1);
      streamData.push({ radius, theta, phi, speed: 0.005 + Math.random() * 0.02 });
      streamPositions[i * 3] = radius * Math.sin(phi) * Math.cos(theta);
      streamPositions[i * 3 + 1] = radius * Math.sin(phi) * Math.sin(theta);
      streamPositions[i * 3 + 2] = radius * Math.cos(phi);
    }
    streamGeo.setAttribute("position", new THREE.BufferAttribute(streamPositions, 3));
    const streamMat = new THREE.PointsMaterial({
      color: 0xffffff, size: 0.022, transparent: true, opacity: 0.6,
      sizeAttenuation: true, depthWrite: false,
    });
    const streams = new THREE.Points(streamGeo, streamMat);
    root.add(streams);

    root.add(crystalRoot);

    // ════════════════════════════════════════════════════════════════════
    //  TUNNEL — torus-knot wireframe
    // ════════════════════════════════════════════════════════════════════
    const tunnelRoot = new THREE.Group();
    const knotGeo = new THREE.TorusKnotGeometry(1.7, 0.42, 180, 18, 2, 3);
    const knotEdges = new THREE.EdgesGeometry(knotGeo);
    const knotMat = new THREE.LineBasicMaterial({
      color: 0xffffff, transparent: true, opacity: 0.7,
    });
    const knot = new THREE.LineSegments(knotEdges, knotMat);
    tunnelRoot.add(knot);
    root.add(tunnelRoot);

    // ════════════════════════════════════════════════════════════════════
    //  GLOBE — wireframe-сфера + 15 lat/lon-точек
    // ════════════════════════════════════════════════════════════════════
    const globeRoot = new THREE.Group();
    const sphereGeo = new THREE.SphereGeometry(2.0, 24, 16);
    const sphereEdges = new THREE.WireframeGeometry(sphereGeo);
    const sphereMat = new THREE.LineBasicMaterial({
      color: 0xffffff, transparent: true, opacity: 0.4,
    });
    const sphere = new THREE.LineSegments(sphereEdges, sphereMat);
    globeRoot.add(sphere);

    // Точки городов (Latitude, Longitude) — даёт ощущение «глобальной сети»
    const COUNTRY_COORDS: [number, number][] = [
      [40, -74], [51, -0.1], [35, 139], [52, 13], [55, 37],
      [1, 103],  [-33, 151], [37, -122], [25, 55],  [19, 73],
      [48, 2],   [60, 24],   [50, 14],   [41, 28],  [43, -79],
    ];
    const dotGeo = new THREE.BufferGeometry();
    const dotPos = new Float32Array(COUNTRY_COORDS.length * 3);
    COUNTRY_COORDS.forEach(([lat, lon], i) => {
      const phi = (90 - lat) * (Math.PI / 180);
      const theta = (lon + 180) * (Math.PI / 180);
      dotPos[i * 3] = -2.05 * Math.sin(phi) * Math.cos(theta);
      dotPos[i * 3 + 1] = 2.05 * Math.cos(phi);
      dotPos[i * 3 + 2] = 2.05 * Math.sin(phi) * Math.sin(theta);
    });
    dotGeo.setAttribute("position", new THREE.BufferAttribute(dotPos, 3));
    const dotMat = new THREE.PointsMaterial({
      color: 0xffffff, size: 0.13, sizeAttenuation: true,
      transparent: true, opacity: 0.95,
    });
    const dots = new THREE.Points(dotGeo, dotMat);
    globeRoot.add(dots);

    root.add(globeRoot);

    // ════════════════════════════════════════════════════════════════════
    //  PARTICLES — поле точек с parallax, без центрального объекта
    // ════════════════════════════════════════════════════════════════════
    // Особый root — не вращается с глобальным rotation, реагирует только
    // на parallax от мыши (см. animate). Так ощущение «звёздного дрейфа»,
    // а не «крутящегося облака».
    const particlesRoot = new THREE.Group();
    const PARTICLE_COUNT = 600;
    const pGeo = new THREE.BufferGeometry();
    const pPos = new Float32Array(PARTICLE_COUNT * 3);
    type PData = { vx: number; vy: number; vz: number };
    const pData: PData[] = [];
    for (let i = 0; i < PARTICLE_COUNT; i++) {
      pPos[i * 3] = (Math.random() - 0.5) * 26;
      pPos[i * 3 + 1] = (Math.random() - 0.5) * 18;
      pPos[i * 3 + 2] = (Math.random() - 0.5) * 22 - 4;
      pData.push({
        vx: (Math.random() - 0.5) * 0.004,
        vy: (Math.random() - 0.5) * 0.003,
        vz: (Math.random() - 0.5) * 0.002,
      });
    }
    pGeo.setAttribute("position", new THREE.BufferAttribute(pPos, 3));
    const pMat = new THREE.PointsMaterial({
      color: 0xffffff, size: 0.05, transparent: true, opacity: 0.7,
      sizeAttenuation: true, depthWrite: false,
    });
    const particles = new THREE.Points(pGeo, pMat);
    particlesRoot.add(particles);
    scene.add(particlesRoot);

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

    // ── Pause when window hidden ─────────────────────────────────────────
    let visible = !document.hidden;
    const onVisibility = () => { visible = !document.hidden; };
    document.addEventListener("visibilitychange", onVisibility);

    // ── Animation ────────────────────────────────────────────────────────
    const clock = new THREE.Clock();
    let raf = 0;
    let glow = 0.4;
    let speed = 0.5;
    const tint = new THREE.Color(0xffffff);
    let time = 0;
    let currentBg: typeof background = "crystal";

    const applySceneVisibility = (bg: typeof background) => {
      crystalRoot.visible = bg === "crystal";
      tunnelRoot.visible = bg === "tunnel";
      globeRoot.visible = bg === "globe";
      streams.visible = bg === "crystal" || bg === "tunnel";
      particlesRoot.visible = bg === "particles";
    };
    applySceneVisibility(currentBg);

    const animate = () => {
      raf = requestAnimationFrame(animate);
      if (!visible) return;
      const dt = clock.getDelta();

      const t = targetRef.current;
      glow += (t.glow - glow) * 0.05;
      speed += (t.speed - speed) * 0.05;
      tint.lerp(t.tint, 0.05);
      (matSolid.color as THREE.Color).lerp(t.solidColor, 0.05);
      fog.color.lerp(t.fogColor, 0.05);

      // Переключение сцены — простое visible-toggle, без disposal.
      if (t.background !== currentBg) {
        currentBg = t.background;
        applySceneVisibility(currentBg);
      }

      time += dt * speed;
      const tm = time;

      mouse.x += (mouseTarget.x - mouse.x) * 0.05;
      mouse.y += (mouseTarget.y - mouse.y) * 0.05;

      // Применяем tint ко всем линиям/частицам.
      wireMat.color.copy(tint);
      streamMat.color.copy(tint);
      knotMat.color.copy(tint);
      sphereMat.color.copy(tint);
      dotMat.color.copy(tint);
      pMat.color.copy(tint);
      shards.forEach(({ orbit }) => orbit.mat.color.copy(tint));
      rings.forEach((r) => (r.material as THREE.MeshBasicMaterial).color.copy(tint));

      streamMat.opacity = 0.3 + glow * 0.7;
      wireMat.opacity = 0.4 + glow * 0.5;
      knotMat.opacity = 0.4 + glow * 0.5;
      sphereMat.opacity = 0.25 + glow * 0.4;
      pMat.opacity = 0.4 + glow * 0.4;

      // ── Вращение root ────────────────────────────────────────────────
      // Для crystal/tunnel/globe — общая идея «объект в воздухе»,
      // root тихо крутится + parallax. Для particles — root неподвижен,
      // частицы получают свой parallax-сдвиг прямо в shader-координатах.
      if (currentBg !== "particles") {
        root.rotation.y = tm * 0.18 + mouse.x * 0.35;
        root.rotation.x = Math.sin(tm * 0.3) * 0.1 + mouse.y * 0.25;
      } else {
        root.rotation.set(0, 0, 0);
      }

      // ── Обновления per-сцена ─────────────────────────────────────────
      if (currentBg === "crystal") {
        inner.rotation.y -= dt * 0.6 * speed;
        inner.rotation.x += dt * 0.3 * speed;
        core.rotation.y += dt * speed;
        const pulse = 1 + Math.sin(tm * 2) * 0.08;
        core.scale.setScalar(pulse * (0.7 + glow * 0.5));

        rings.forEach((ring, i) => {
          ring.rotation.z += dt * (0.1 + i * 0.05) * speed;
          ring.rotation.x += dt * 0.05 * speed;
        });

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
      }

      if (currentBg === "tunnel") {
        knot.rotation.x += dt * 0.3 * speed;
        knot.rotation.y += dt * 0.2 * speed;
      }

      if (currentBg === "globe") {
        // Глобус крутится только вокруг Y — как настоящая планета.
        globeRoot.rotation.y += dt * 0.15 * speed;
      }

      if (currentBg === "particles") {
        // Дрейф каждой частицы + лёгкий parallax-сдвиг всего поля от мыши.
        const positions = particles.geometry.attributes.position.array as Float32Array;
        for (let i = 0; i < PARTICLE_COUNT; i++) {
          const d = pData[i];
          positions[i * 3]     += d.vx * speed * 60 * dt;
          positions[i * 3 + 1] += d.vy * speed * 60 * dt;
          positions[i * 3 + 2] += d.vz * speed * 60 * dt;
          // wrap по куборегиону
          if (positions[i * 3] > 13)  positions[i * 3] -= 26;
          if (positions[i * 3] < -13) positions[i * 3] += 26;
          if (positions[i * 3 + 1] > 9)  positions[i * 3 + 1] -= 18;
          if (positions[i * 3 + 1] < -9) positions[i * 3 + 1] += 18;
          if (positions[i * 3 + 2] > 7)  positions[i * 3 + 2] -= 22;
          if (positions[i * 3 + 2] < -15) positions[i * 3 + 2] += 22;
        }
        particles.geometry.attributes.position.needsUpdate = true;
        particlesRoot.position.x = mouse.x * 1.4;
        particlesRoot.position.y = mouse.y * 0.9;
      } else {
        particlesRoot.position.set(0, 0, 0);
      }

      // Streams (общие для crystal/tunnel) — летят к центру.
      if (currentBg === "crystal" || currentBg === "tunnel") {
        const positions = streams.geometry.attributes.position.array as Float32Array;
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
      }

      renderer.render(scene, camera);
    };
    animate();

    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("resize", onResize);
      document.removeEventListener("visibilitychange", onVisibility);

      crystalGeo.dispose();
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

      knotGeo.dispose();
      knotEdges.dispose();
      knotMat.dispose();

      sphereGeo.dispose();
      sphereEdges.dispose();
      sphereMat.dispose();
      dotGeo.dispose();
      dotMat.dispose();

      pGeo.dispose();
      pMat.dispose();

      renderer.dispose();
    };
  }, []);

  return <canvas ref={canvasRef} className="scene-canvas" />;
}
