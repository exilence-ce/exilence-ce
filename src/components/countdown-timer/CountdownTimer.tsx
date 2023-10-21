import { observer } from 'mobx-react-lite';
import moment from 'moment';
import React, { useRef } from 'react';
import { useEffect, useState } from 'react';

type CountdownTimerProps = {
  comparison: number;
  timeOverCb?: () => void;
};

const CountdownTimer = ({ comparison, timeOverCb }: CountdownTimerProps) => {
  const timeoutRef = useRef<NodeJS.Timeout>();
  const calculateTimeLeft = () => {
    const difference = moment.utc(comparison).diff(moment.utc());
    let timeLeft = 0;

    if (difference > 0) {
      timeLeft = Math.floor(difference / 1000);
    } else {
      timeOverCb?.();
    }

    return moment.duration(timeLeft, 'seconds').format('m [minutes], s [seconds]');
  };

  const [timeLeft, setTimeLeft] = useState(() => calculateTimeLeft());

  useEffect(() => {
    timeoutRef.current = setTimeout(() => {
      setTimeLeft(calculateTimeLeft());
    }, 1000);
  });

  useEffect(() => {
    return () => {
      timeoutRef.current && clearTimeout(timeoutRef.current);
    };
  }, []);

  return <>{timeLeft}</>;
};

export default observer(CountdownTimer);
