import { app, Tray, Menu, shell, BrowserWindow } from 'electron';
import { checkForUpdates } from './autoUpdater';
import { destroyNetWorthOverlay } from './overlays/NetWorth/netWorthOverlay';
import * as path from 'path';
import { SYSTEMS } from '../enums';

const trayIconPath = path.join(__dirname, `../../icon512x512.png`);
const trayIconMacPath = path.join(__dirname, `../../tray_mac.png`);
import checkForMissingWindow from './utils';

type CreateTrayProps = {
  mainWindow: BrowserWindow;
  updateAvailable: boolean;
  isQuittingCallback: (status: boolean) => void;
};

let tray: Tray | null;

const createTray = ({ mainWindow, updateAvailable, isQuittingCallback }: CreateTrayProps) => {
  checkForMissingWindow({ category: 'tray', mainWindow });
  if(process.platform === SYSTEMS.MACOS) 
    tray = new Tray(trayIconMacPath);
  else
    tray = new Tray(trayIconPath);

  const contextMenu = Menu.buildFromTemplate([
    {
      label: 'Show Exilence CE',
      click: () => mainWindow.show(),
    },
    {
      label: 'Check for Updates...',
      sublabel: updateAvailable ? 'Update available!' : 'No updates available',
      click: async () => {
        await checkForUpdates();
      },
    },
    { type: 'separator' },
    {
      label: 'Donate',
      type: 'normal',
      click: async () => {
        await shell.openExternal('https://ko-fi.com/ExilenceCE');
      },
    },
    {
      label: 'Discord / Help',
      type: 'normal',
      click: async () => {
        await shell.openExternal('https://discord.gg/2T3WXBgjaM');
      },
    },
    { type: 'separator' },
    {
      label: 'Quit',
      type: 'normal',
      click: () => {
        isQuittingCallback(true);
        destroyNetWorthOverlay();
        app.quit();
      },
    },
  ]);

  tray.setToolTip('Exilence CE');
  tray.on('click', () => mainWindow.show());
  tray.setIgnoreDoubleClickEvents(true);
  tray.setContextMenu(contextMenu);
};

const destroyTray = () => tray instanceof Tray && tray.destroy();

export { createTray, CreateTrayProps, destroyTray };
