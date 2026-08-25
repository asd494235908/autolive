type GateUser = {
  id: string;
  status: string;
};

type GateDevice = {
  id: string;
  user_id: string;
  status: string;
  activation_expires_at?: string | null;
};

type GateProfile = {
  user: GateUser;
  device: GateDevice;
};

export function isConfirmedDesktopAccess(
  user: GateUser | null,
  device: GateDevice | null,
  expectedDeviceId: string,
  now = Date.now(),
): boolean {
  if (!expectedDeviceId) return false;
  if (!user || user.status !== 'active') return false;
  if (
    !device
    || device.user_id !== user.id
    || device.id !== expectedDeviceId
    || device.status !== 'active'
  ) return false;
  if (typeof device.activation_expires_at !== 'string') return false;
  const expiresAt = Date.parse(device.activation_expires_at);
  return Number.isFinite(expiresAt) && expiresAt > now;
}

export function shouldAutomaticallyRegisterDevice(
  profile: GateProfile,
  expectedDeviceId: string,
  now = Date.now(),
): boolean {
  if (!expectedDeviceId) return false;
  if (profile.user.status !== 'active') return false;
  if (profile.device.id !== expectedDeviceId || profile.device.user_id !== profile.user.id) return false;
  if (profile.device.status === 'pending_activation') return true;
  if (profile.device.status !== 'active') return false;
  const expiresAt = Date.parse(profile.device.activation_expires_at ?? '');
  return Number.isFinite(expiresAt) && expiresAt <= now;
}
