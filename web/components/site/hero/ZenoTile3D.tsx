"use client";

import { Float, Outlines, RoundedBox } from "@react-three/drei";
import { Canvas, useFrame } from "@react-three/fiber";
import { useMemo, useRef } from "react";
import * as THREE from "three";

const SAFFRON = "#efad48";
const INK = "#17140f";

/**
 * The one 3D object on the site: a clay tile with a play mark, outlined in ink so it belongs to
 * the same drawn world as the guide and the line. It turns a little towards the pointer.
 */
function Tile() {
  const group = useRef<THREE.Group>(null!);
  const play = useMemo(() => {
    const shape = new THREE.Shape();
    shape.moveTo(-0.2, -0.27);
    shape.lineTo(0.3, 0);
    shape.lineTo(-0.2, 0.27);
    shape.closePath();
    return shape;
  }, []);

  useFrame((state, delta) => {
    const g = group.current;
    g.rotation.y = THREE.MathUtils.damp(g.rotation.y, state.pointer.x * 0.5 - 0.32, 3.2, delta);
    g.rotation.x = THREE.MathUtils.damp(g.rotation.x, -state.pointer.y * 0.32 + 0.12, 3.2, delta);
  });

  return (
    <group ref={group}>
      <RoundedBox args={[1.7, 1.7, 0.46]} radius={0.24} smoothness={8}>
        <meshStandardMaterial color={SAFFRON} roughness={0.55} metalness={0} />
        <Outlines thickness={0.05} color={INK} />
      </RoundedBox>
      <mesh position={[0.02, 0, 0.21]}>
        <extrudeGeometry
          args={[play, { depth: 0.07, bevelEnabled: true, bevelSize: 0.022, bevelThickness: 0.022, bevelSegments: 5 }]}
        />
        <meshStandardMaterial color={INK} roughness={0.5} />
      </mesh>
    </group>
  );
}

export default function ZenoTile3D({ active }: { active: boolean }) {
  return (
    <Canvas
      dpr={[1, 1.8]}
      frameloop={active ? "always" : "never"}
      camera={{ position: [0, 0, 4.4], fov: 34 }}
      gl={{ antialias: true, alpha: true, powerPreference: "low-power" }}
      aria-hidden="true"
    >
      <ambientLight intensity={0.55} color="#fff6e6" />
      <hemisphereLight args={["#fff6e6", "#c9b48f", 1.25]} />
      <directionalLight position={[3, 4, 5]} intensity={2.1} color="#fff4e2" />
      <directionalLight position={[-4, -3, 2]} intensity={0.35} color={SAFFRON} />
      <Float speed={1.4} rotationIntensity={0.2} floatIntensity={0.55}>
        <Tile />
      </Float>
    </Canvas>
  );
}
