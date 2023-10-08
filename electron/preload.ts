import * as os from 'os';

declare global {
  interface Window { OsDetection: any; }
}

window.OsDetection = window.OsDetection || {};
window.OsDetection = {
  isWin: () => os.platform() === 'win32',
  isMac: () => os.platform() === 'darwin',
  isLinux: () => os.platform() === 'linux'
}