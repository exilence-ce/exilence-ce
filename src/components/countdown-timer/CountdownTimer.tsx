import { Typography } from '@mui/material';
import { observer } from 'mobx-react-lite';
import moment from 'moment';
import React from 'react';
import { useEffect, useState } from 'react';

type CountdownTimerProps = {
  comparison: number;
};

const CountdownTimer = ({ comparison }: CountdownTimerProps) => {
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
    const timeout = setTimeout(() => {
      setTimeLeft(calculateTimeLeft());
    }, 1000);
    return () => clearTimeout(timeout);
  });

  return <Typography variant="caption">{timeLeft} seconds</Typography>;
};

export default observer(CountdownTimer);
