export async function sendMessage(content: string, baseUrl = ''): Promise<boolean> {
  const res = await fetch(`${baseUrl}/api/message`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  });
  return res.ok;
}

export interface SpawnFleetResult {
  accepted: boolean;
  fleet_id: string;
  workflow_id: string;
  spec: string;
  members: string[];
}

export async function spawnFleet(spec: string, baseUrl = ''): Promise<SpawnFleetResult | null> {
  const res = await fetch(`${baseUrl}/api/fleet/spawn`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ spec }),
  });
  if (!res.ok) return null;
  return (await res.json()) as SpawnFleetResult;
}

export async function startDebate(topic: string, baseUrl = ''): Promise<boolean> {
  const res = await fetch(`${baseUrl}/api/debate/start`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ topic }),
  });
  return res.ok;
}
