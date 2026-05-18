import { IStashTab } from '../interfaces/stash.interface';

export function getStashTabRequestId(stashTab: IStashTab): string {
  return stashTab.parent ? `${stashTab.parent}/${stashTab.id}` : stashTab.id;
}

export function getRequestedStashTab(
  responseStashTab: IStashTab,
  requestedStashTab: IStashTab
): IStashTab {
  if (!requestedStashTab.parent || responseStashTab.id !== requestedStashTab.parent) {
    return responseStashTab;
  }

  return (
    responseStashTab.children?.find((child) => child.id === requestedStashTab.id) ??
    responseStashTab
  );
}
