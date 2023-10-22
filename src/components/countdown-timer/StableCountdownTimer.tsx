import { observer } from 'mobx-react-lite';
import moment from 'moment';
import React, { useCallback, useRef } from 'react';
import { useEffect, useState } from 'react';

type StableCountdownTimerProps = {
  comparison: number;
  timeOverCb?: () => void;
};

const StableCountdownTimer = ({ comparison, timeOverCb }: StableCountdownTimerProps) => {
  const comparisonRef = useRef(comparison);
  comparisonRef.current = comparison;

  const calculateTimeLeft = useCallback(() => {
    const difference = moment.utc(comparisonRef.current).diff(moment.utc());
    let timeLeft = 0;

    if (difference > 0) {
      timeLeft = Math.round(difference / 1000);
    } else {
      timeOverCb?.();
    }

    return moment.duration(timeLeft, 'seconds').format('m [minutes], s [seconds]');
  }, []);

  const [timeLeft, setTimeLeft] = useState(() => calculateTimeLeft());

  useEffect(() => {
    const interval = setInterval(() => {
      setTimeLeft(calculateTimeLeft());
    }, 1000);
    return () => clearInterval(interval);
  }, [calculateTimeLeft]);

  return <>{timeLeft}</>;
};

export default observer(StableCountdownTimer);
