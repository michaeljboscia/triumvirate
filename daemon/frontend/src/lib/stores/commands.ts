export async function sendMessage(content: string, baseUrl = ''): Promise<boolean> {
  const res = await fetch(`${baseUrl}/api/message`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ content }),
  });
  return res.ok;
}

export async function spawnFleet(spec: string, baseUrl = ''): Promise<boolean> {
  const res = await fetch(`${baseUrl}/api/fleet/spawn`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ spec }),
  });
  return res.ok;
}

export async function startDebate(topic: string, baseUrl = ''): Promise<boolean> {
  const res = await fetch(`${baseUrl}/api/debate/start`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ topic }),
  });
  return res.ok;
}
