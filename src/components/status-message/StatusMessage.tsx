import React from 'react';
import { useTranslation } from 'react-i18next';
import { Stack, Tooltip, Typography } from '@mui/material';
import { observer } from 'mobx-react-lite';
import InfoIcon from '@mui/icons-material/Info';
import { IStatusMessage } from '../../interfaces/status-message.interface';
import useStyles from './StatusMessage.styles';
import moment from 'moment';
import { rootStore } from '../..';
import CountdownTimer from '../countdown-timer/CountdownTimer';

type StatusMessageProps = {
  statusMessage?: IStatusMessage;
  infoLabel?: string;
  isSnapshotting?: boolean;
  estimatedSnapshotTime: {
    estimated: number;
    estimatedStatic?: moment.Duration;
  };
  isNextSnapshotWithoutWaitTimeMsg?: boolean;
};

const StatusMessage = ({
  statusMessage,
  infoLabel,
  isSnapshotting,
  estimatedSnapshotTime,
  isNextSnapshotWithoutWaitTimeMsg,
}: StatusMessageProps) => {
  const { t } = useTranslation();
  const classes = useStyles();

  return (
    <>
      {statusMessage && (
        <Stack spacing={1} direction="row" alignItems="center">
          {isNextSnapshotWithoutWaitTimeMsg ? (
            <>
              <Typography variant="body2">
                {`${t(`status:message.${statusMessage.message}`)} `}
                <CountdownTimer
                  comparison={estimatedSnapshotTime.estimated}
                  timeOverCb={() => {
                    rootStore.rateLimitStore.setEstimatedSnapshotTime();
                  }}
                />
              </Typography>
            </>
          ) : (
            <Typography variant="body2">
              {`${t(`status:message.${statusMessage.message}`, {
                param: statusMessage?.translateParam,
              })} `}
              {statusMessage.currentCount && statusMessage.totalCount && (
                <>
                  {statusMessage.currentCount} / {statusMessage.totalCount}
                </>
              )}
              {' ...'}
            </Typography>
          )}
          {isSnapshotting && (
            <>
              <Typography variant="body2">
                {` ${t('status:message.estimated_time')} `}
                <CountdownTimer comparison={estimatedSnapshotTime.estimated} />
              </Typography>

              {` `}
            </>
          )}
          {infoLabel && isSnapshotting && (
            <Tooltip title={infoLabel || ''} placement="bottom">
              <InfoIcon classes={{ root: classes.iconRoot }} />
            </Tooltip>
          )}
        </Stack>
      )}
    </>
  );
};

export default observer(StatusMessage);
