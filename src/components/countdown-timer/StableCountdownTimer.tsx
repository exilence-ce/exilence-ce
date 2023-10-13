import { Typography } from '@mui/material';
import { observer } from 'mobx-react-lite';
import moment from 'moment';
import React from 'react';
import { useEffect, useState } from 'react';

type StableCountdownTimerProps = {
  comparison: number;
};

const StableCountdownTimer = ({ comparison }: StableCountdownTimerProps) => {
  const calculateTimeLeft = () => {
    const difference = moment.utc(comparison).diff(moment.utc());
    let timeLeft = 0;

    if (difference > 0) {
      timeLeft = Math.floor(difference / 1000);
    }

    return timeLeft;
  };

  const [timeLeft, setTimeLeft] = useState(calculateTimeLeft());

  useEffect(() => {
    const interval = setInterval(() => {
      setTimeLeft(calculateTimeLeft());
    }, 1000);
    return () => clearInterval(interval);
  }, []);

  return <Typography variant="body2">{timeLeft} seconds</Typography>;
};

export default observer(StableCountdownTimer);
