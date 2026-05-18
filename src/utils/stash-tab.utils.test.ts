import { IStashTab } from '../interfaces/stash.interface';
import { getRequestedStashTab, getStashTabRequestId } from './stash-tab.utils';

const createStashTab = (id: string, parent?: string): IStashTab => ({
  id,
  parent,
  index: 0,
  name: 'Dump',
  type: 'PremiumStash',
  metadata: {},
});

describe('getStashTabRequestId', () => {
  it('uses the tab id for top-level stash tabs', () => {
    expect(getStashTabRequestId(createStashTab('aaaa1111'))).toBe('aaaa1111');
  });

  it('uses the documented parent/substash path for child stash tabs', () => {
    expect(getStashTabRequestId(createStashTab('bbbb2222', 'aaaa1111'))).toBe('aaaa1111/bbbb2222');
  });
});

describe('getRequestedStashTab', () => {
  it('returns top-level stash responses unchanged', () => {
    const stashTab = createStashTab('aaaa1111');

    expect(getRequestedStashTab(stashTab, stashTab)).toEqual(stashTab);
  });

  it('unwraps child stash tabs from parent-wrapped API responses', () => {
    const requestedChild = createStashTab('bbbb2222', 'aaaa1111');
    const parentResponse = {
      ...createStashTab('aaaa1111'),
      children: [requestedChild],
    };

    expect(getRequestedStashTab(parentResponse, requestedChild)).toEqual(requestedChild);
  });
});
