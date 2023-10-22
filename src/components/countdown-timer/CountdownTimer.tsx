import { observer } from 'mobx-react-lite';
import moment from 'moment';
import React, { useRef } from 'react';
import { useEffect, useState } from 'react';

type CountdownTimerProps = {
  comparison: number;
  timeOverCb?: () => void;
};

const CountdownTimer = ({ comparison, timeOverCb }: CountdownTimerProps) => {
  const timeoutRef = useRef<NodeJS.Timeout[]>([]);
  const calculateTimeLeft = () => {
    const difference = moment.utc(comparison).diff(moment.utc());
    let timeLeft = 0;

    if (difference > 0) {
      timeLeft = Math.round(difference / 1000);
    } else {
      timeOverCb?.();
    }

    return moment.duration(timeLeft, 'seconds').format('m [minutes], s [seconds]');
  };

  const [timeLeft, setTimeLeft] = useState(() => calculateTimeLeft());

  useEffect(() => {
    const timeout = setTimeout(() => {
      const index = timeoutRef.current.indexOf(timeout);
      if (index > -1) {
        timeoutRef.current.splice(index, 1);
      }
      setTimeLeft(calculateTimeLeft());
    }, 1000);
    timeoutRef.current.push(timeout);
  });

  useEffect(
    () => () => {
      timeoutRef.current.forEach((timeout) => clearTimeout(timeout));
    },
    []
  );

  return <>{timeLeft}</>;
};

export default observer(CountdownTimer);
